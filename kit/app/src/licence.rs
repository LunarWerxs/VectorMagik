//! The commercial licence: VectorMagik is free for personal and other
//! noncommercial use (PolyForm Noncommercial 1.0.0), and business use takes a
//! licence, US$9 a month per seat, sold through Connections Pay the way
//! SageThumbs 2K sells its own.
//!
//! What the buyer gets is a licence key (`esk_XXXXX-XXXXX-XXXXX-XXXXX`, one per
//! seat, emailed by Pay). The app redeems it once at Pay's public seat door,
//! through the site's own Worker (`REDEEM_URL`), and keeps the certificate Pay
//! hands back: a signed statement that this seat holds a licence, verified
//! here against Pay's public key with no network at all. A certificate lives
//! 30 days; re-presenting the same key is Pay's designed refresh path (a
//! replay answers with a fresh one), so the app re-redeems when its
//! certificate nears its end, and a seat whose subscription has ended is
//! refused there and the licence goes with it. There is no relay holding a
//! merchant key, because the key IS the credential.
//!
//! Nothing here stops the app: a licence is shown, never enforced. Every
//! failure reads as "no certificate", and the app says Personal use.
//!
//! The seat's subject is derived from its key (`subject`), so a person's seat
//! follows them to each of their computers and to the browser, and clearing a
//! browser's storage can never strand it on a subject nobody holds any more.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Pay's Ed25519 public key, the raw 32 bytes (the tail of the SPKI DER that
/// `GET /api/licences/verification-key` serves). The same key signs every
/// product's certificates; `PRODUCT_ID` tells ours apart. Copied from
/// SageThumbs 2K's `licence_cert.rs`, which verified it against both encodings
/// the endpoint publishes on 2026-09-04. There is no key id, so a rotation
/// looks like a bad signature, which reads as Personal use until a release
/// carries the new key.
const VERIFY_KEY: [u8; 32] = [
    0xed, 0x07, 0xd3, 0x99, 0xe3, 0xd6, 0xe9, 0x26, 0xd1, 0x0d, 0x7b, 0xdd, 0x8e, 0x5d, 0x6d, 0x1d,
    0x13, 0x19, 0x62, 0x91, 0xd9, 0xfd, 0xd3, 0x61, 0xbb, 0x65, 0xc8, 0xc0, 0x98, 0x6e, 0x67, 0xac,
];

/// The audience every licence certificate carries; the same key signs other
/// short-lived tokens, which must never verify as a licence.
const AUDIENCE: &str = "connections-licence";

/// Our Pay catalog product: "VectorMagik Commercial License (Monthly)",
/// US$9 a month per seat, SKU VM-COMM-MONTHLY, sold under the LunarWerx
/// fleet's merchant (the `connections` workspace, created September 24,
/// 2026; docs/TODO.md, the licensing item). `None` would take licences off
/// sale: no certificate would be ours and the card would offer no redemption.
pub const PRODUCT_ID: Option<&str> = Some("48f44398-36ab-4d53-a453-9d6ebe4abdae");

/// Where a key is redeemed: the site's Worker, which forwards to Pay's public
/// seat door (so the browser build calls its own origin, and a move of Pay's
/// host is a Worker variable, not a release).
pub const REDEEM_URL: &str = "https://vectormagik.lunarwerx.com/api/license/redeem";
/// The site's licence section: the price, what counts as commercial use, and
/// the Buy button once licences are on sale.
pub const LICENCE_PAGE: &str = "https://vectormagik.lunarwerx.com/#license";
/// Where a licence is bought: a redirect the site's Worker keeps pointed at
/// Pay's hosted checkout for `PRODUCT_ID`.
pub const BUY_URL: &str = "https://vectormagik.lunarwerx.com/license/buy";
/// Where a buyer manages or cancels the subscription (a redirect the site's
/// Worker keeps pointed at Pay's customer page).
pub const MANAGE_URL: &str = "https://vectormagik.lunarwerx.com/license/manage";
/// The price, as every surface states it.
pub const PRICE: &str = "US$9 a month per person";

/// A certificate this close to its end is renewed at the next chance.
pub const RENEW_WITHIN_SECONDS: i64 = 10 * 24 * 3600;

/// What a good certificate says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// Pay's licence id (`lic`).
    pub licence_id: String,
    /// How many seats the licence holds (`units`).
    pub seats: i64,
    /// When the certificate stops proving the licence, Unix seconds (`exp`).
    pub expires: i64,
}

/// Why a certificate did not verify; every one reads as "no certificate".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertError {
    /// Not `<payload>.<signature>`, or not unpadded base64url.
    Malformed,
    /// Ed25519 said no (or Pay's key has rotated).
    BadSignature,
    /// Signed, but not a licence we can read.
    BadPayload,
    /// A licence for another product, or another seat's subject.
    NotOurs,
    /// Past its `exp`: re-redeem the key.
    Expired,
}

/// Turn what someone typed or pasted into the canonical key
/// `esk_XXXXX-XXXXX-XXXXX-XXXXX`, or `None` when it is not shaped like one:
/// surrounding space, case and dashes are forgiven, nothing else (the same
/// rule as SageThumbs' `normalize_key`, so a key works in both).
pub fn normalize_key(raw: &str) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("esk_")?;
    let body: String = rest.chars().filter(|&c| c != '-').collect();
    if body.len() != 20 || !body.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let upper = body.to_ascii_uppercase();
    Some(format!(
        "esk_{}-{}-{}-{}",
        &upper[0..5],
        &upper[5..10],
        &upper[10..15],
        &upper[15..20]
    ))
}

/// The key as it may be shown: its prefix and first group only.
pub fn key_hint(key: &str) -> String {
    format!("{}\u{2026}", &key[..key.len().min(9)])
}

/// The seat's subject: 64 hex digits derived from its canonical key, so a
/// person's seat is the same seat on every computer they use.
pub fn subject(key: &str) -> String {
    let digest = Sha256::digest(format!("vectormagik-seat:{key}").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The body the redeem route takes.
pub fn redeem_body(key: &str) -> String {
    serde_json::json!({ "key": key, "subject": subject(key) }).to_string()
}

/// What a redeem answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Redeemed {
    /// A certificate, fresh from Pay.
    Certificate(String),
    /// Pay refused the key (unknown, revoked, ended); the licence goes, with
    /// this reason for the card.
    Refused(String),
    /// No answer worth acting on (offline, a server error): keep what we have.
    Unreachable(String),
}

/// Read the redeem route's reply: `status` 0 means no response at all.
pub fn read_redeem(status: u16, body: &str) -> Redeemed {
    let value: Option<Value> = serde_json::from_str(body).ok();
    let field = |name: &str| {
        value
            .as_ref()
            .and_then(|v| v.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    match status {
        200 => match field("certificate") {
            Some(certificate) if !certificate.is_empty() => Redeemed::Certificate(certificate),
            _ => Redeemed::Unreachable(field("certificateError").map_or_else(
                || "Pay accepted the key but sent no certificate.".to_owned(),
                |error| format!("Pay accepted the key but sent no certificate ({error})."),
            )),
        },
        // A definite no from Pay about this key.
        400..=499 if status != 408 && status != 429 => Redeemed::Refused(
            field("message")
                .or_else(|| field("error"))
                .unwrap_or_else(|| format!("The key was refused ({status}).")),
        ),
        0 => Redeemed::Unreachable("No connection to the license server.".to_owned()),
        _ => Redeemed::Unreachable(format!(
            "The license server answered {status}; try again later."
        )),
    }
}

/// Verify `cert` for `product` and the seat `subject` at `now` (Unix seconds).
pub fn verify(cert: &str, product: &str, subject: &str, now: i64) -> Result<Verified, CertError> {
    let (payload, signature) = cert.trim().split_once('.').ok_or(CertError::Malformed)?;
    if payload.is_empty() || signature.is_empty() || signature.contains('.') {
        return Err(CertError::Malformed);
    }
    let signature: [u8; 64] = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| CertError::Malformed)?
        .as_slice()
        .try_into()
        .map_err(|_| CertError::Malformed)?;
    let key = VerifyingKey::from_bytes(&VERIFY_KEY).map_err(|_| CertError::BadSignature)?;
    // Signed over the payload's base64url TEXT, not the decoded JSON.
    key.verify(payload.as_bytes(), &Signature::from_bytes(&signature))
        .map_err(|_| CertError::BadSignature)?;
    let json = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| CertError::Malformed)?;
    let claims: Value = serde_json::from_slice(&json).map_err(|_| CertError::BadPayload)?;
    let text = |name: &str| claims.get(name).and_then(Value::as_str);
    let (Some(aud), Some(claimed_product), Some(sub), Some(licence_id), Some(expires)) = (
        text("aud"),
        text("product"),
        text("sub"),
        text("lic"),
        claims.get("exp").and_then(Value::as_i64),
    ) else {
        return Err(CertError::BadPayload);
    };
    if aud != AUDIENCE {
        return Err(CertError::BadPayload);
    }
    if claimed_product != product || sub != subject {
        return Err(CertError::NotOurs);
    }
    if expires <= now {
        return Err(CertError::Expired);
    }
    Ok(Verified {
        licence_id: licence_id.to_owned(),
        seats: claims.get("units").and_then(Value::as_i64).unwrap_or(1),
        expires,
    })
}

/// The licence as stored with the preferences: the key and its latest
/// certificate (empty until one arrives).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stored {
    pub key: String,
    pub certificate: String,
}

/// What the Licence card shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
    /// No key: free for personal and other noncommercial use.
    Personal,
    /// A key whose certificate proves the licence now.
    Commercial(Verified),
    /// A key with no certificate proving it now (not yet redeemed, expired
    /// while offline, or not ours): the app asks Pay again when it can.
    Unproven,
}

/// The standing `stored` gives at `now`.
pub fn standing(stored: &Stored, now: i64) -> Standing {
    let (Some(product), false) = (PRODUCT_ID, stored.key.is_empty()) else {
        return Standing::Personal;
    };
    match verify(&stored.certificate, product, &subject(&stored.key), now) {
        Ok(verified) => Standing::Commercial(verified),
        Err(_) => Standing::Unproven,
    }
}

/// Whether the stored key should be presented to Pay again now: it has no
/// certificate that proves it, or the one it has ends within
/// `RENEW_WITHIN_SECONDS`.
pub fn wants_redeem(stored: &Stored, now: i64) -> bool {
    match standing(stored, now) {
        Standing::Personal => false,
        Standing::Commercial(verified) => verified.expires - now < RENEW_WITHIN_SECONDS,
        Standing::Unproven => true,
    }
}

/// A date for the card, from Unix seconds (UTC, `YYYY-MM-DD`).
pub fn date(unix: i64) -> String {
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let days = unix.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real certificate Pay minted on 2026-09-04 for SageThumbs 2K's product,
    /// signed by the production key (SageThumbs' `licence_cert.rs` fixture):
    /// it pins the encoding and the signature against what Pay emits.
    const REAL_CERT: &str = concat!(
        "eyJhdWQiOiJjb25uZWN0aW9ucy1saWNlbmNlIiwibGljIjoiMWQxZjNkMjktODM5OS00YTY5LTk4ZTgt",
        "ZmI1Y2ViNWI1M2E2IiwicHJvZHVjdCI6IjI0NTQ0NDYxLTk1MzAtNGVkYi04NGU1LTRmMzQ3MTg3NmQ5",
        "OCIsInN1YiI6ImNlMTIwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAw",
        "MDAwMDAwMDAwMGNhZmUiLCJ1bml0IjoiaW5zdGFsbGF0aW9uIiwidGVybSI6InBlcnBldHVhbF9wbHVz",
        "X21haW50ZW5hbmNlIiwidW5pdHMiOjEsInNlYXQiOiJjMTE2YTZiMy04M2QzLTRjYzQtOTE3MC1kMmE4",
        "ZDUwNzg2NGEiLCJtYWludCI6MTgxOTk3Nzk1NSwiY2VpbCI6bnVsbCwiaWF0IjoxNzg4NTM4OTc0LCJl",
        "eHAiOjE3OTExMzA5NzR9.",
        "N15ul_HgXnzpmUohjFFKD5yIW5UwlNdR4t8E_bYjChPnsncvHCxoiB3ldyMDfwF28CHHq-ddB2NymBlx",
        "EH6jAg"
    );
    const REAL_PRODUCT: &str = "24544461-9530-4edb-84e5-4f3471876d98";
    const REAL_SUB: &str = "ce1200000000000000000000000000000000000000000000000000000000cafe";
    const INSIDE: i64 = 1_788_600_000;

    #[test]
    fn a_real_certificate_verifies_for_its_product_and_seat_only() {
        let verified = verify(REAL_CERT, REAL_PRODUCT, REAL_SUB, INSIDE).unwrap();
        assert_eq!(verified.licence_id, "1d1f3d29-8399-4a69-98e8-fb5ceb5b53a6");
        assert_eq!((verified.seats, verified.expires), (1, 1_791_130_974));
        // Another product's certificate, or another seat's, is not ours.
        let other = "00000000-0000-0000-0000-000000000000";
        assert_eq!(
            verify(REAL_CERT, other, REAL_SUB, INSIDE),
            Err(CertError::NotOurs)
        );
        assert_eq!(
            verify(REAL_CERT, REAL_PRODUCT, "deadbeef", INSIDE),
            Err(CertError::NotOurs)
        );
        assert_eq!(
            verify(REAL_CERT, REAL_PRODUCT, REAL_SUB, 1_791_130_974),
            Err(CertError::Expired)
        );
    }

    #[test]
    fn a_tampered_or_malformed_certificate_is_refused() {
        let (payload, signature) = REAL_CERT.split_once('.').unwrap();
        let flip = |s: &str| {
            let mut t = s.to_owned();
            t.replace_range(
                0..1,
                if s.starts_with('e') || s.starts_with('N') {
                    "M"
                } else {
                    "N"
                },
            );
            t
        };
        assert_eq!(
            verify(
                &format!("{}.{signature}", flip(payload)),
                REAL_PRODUCT,
                REAL_SUB,
                INSIDE
            ),
            Err(CertError::BadSignature)
        );
        assert!(verify(
            &format!("{payload}.{}", flip(signature)),
            REAL_PRODUCT,
            REAL_SUB,
            INSIDE
        )
        .is_err());
        for junk in [
            "",
            ".",
            "no-dot",
            "a.b",
            "....",
            "!!!.!!!",
            &format!("{payload}=.{signature}"),
        ] {
            assert!(
                verify(junk, REAL_PRODUCT, REAL_SUB, INSIDE).is_err(),
                "{junk:?}"
            );
        }
    }

    #[test]
    fn keys_are_forgiven_their_case_space_and_dashes_and_nothing_else() {
        let canonical = "esk_ABCDE-FGHIJ-KLMNO-PQRS1";
        assert_eq!(
            normalize_key("  esk_abcdefghijklmnopqrs1 ").as_deref(),
            Some(canonical)
        );
        assert_eq!(
            normalize_key("ESK_ABCDE-FGHIJ-KLMNO-PQRS1").as_deref(),
            Some(canonical)
        );
        for bad in [
            "",
            "esk_",
            "abcdefghijklmnopqrst",
            "esk_ABCDE-FGHIJ-KLMNO-PQRS",
            "esk_ABCDE-FGHIJ-KLMNO-PQRS12",
            "esk_ABCDE-FGHIJ-KLMNO-PQR!1",
        ] {
            assert_eq!(normalize_key(bad), None, "{bad:?}");
        }
        assert_eq!(key_hint(canonical), "esk_ABCDE\u{2026}");
        // A seat's subject is the key's, 64 hex digits, the same everywhere.
        let subject = subject(canonical);
        assert_eq!(subject.len(), 64);
        assert!(subject.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(subject, super::subject("esk_ABCDE-FGHIJ-KLMNO-PQRS2"));
        assert!(redeem_body(canonical).contains(&subject));
    }

    #[test]
    fn a_redeem_reply_is_a_certificate_a_refusal_or_nothing_to_act_on() {
        assert_eq!(
            read_redeem(200, r#"{"ok":true,"replayed":true,"certificate":"a.b"}"#),
            Redeemed::Certificate("a.b".into())
        );
        assert!(matches!(
            read_redeem(
                200,
                r#"{"ok":true,"certificate":null,"certificateError":"x"}"#
            ),
            Redeemed::Unreachable(_)
        ));
        assert_eq!(
            read_redeem(
                404,
                r#"{"error":"unknown_or_expired_key","message":"That key does not exist, has expired, or has been revoked."}"#
            ),
            Redeemed::Refused("That key does not exist, has expired, or has been revoked.".into())
        );
        for status in [0, 408, 429, 500, 502, 503] {
            assert!(
                matches!(read_redeem(status, "oops"), Redeemed::Unreachable(_)),
                "{status}"
            );
        }
    }

    #[test]
    fn another_products_certificate_proves_nothing_and_the_key_goes_back_to_pay() {
        let stored = Stored {
            key: "esk_ABCDE-FGHIJ-KLMNO-PQRS1".into(),
            certificate: REAL_CERT.into(),
        };
        assert!(PRODUCT_ID.is_some_and(|ours| ours != REAL_PRODUCT));
        assert_eq!(standing(&stored, INSIDE), Standing::Unproven);
        assert!(wants_redeem(&stored, INSIDE));
        assert_eq!(standing(&Stored::default(), INSIDE), Standing::Personal);
    }

    #[test]
    fn dates_are_civil_utc() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_791_130_974), "2026-10-04");
        assert_eq!(date(951_782_400), "2000-02-29");
    }
}
