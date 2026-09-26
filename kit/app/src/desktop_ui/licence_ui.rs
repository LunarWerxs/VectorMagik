//! Part of `desktop_ui`: the Licence card and the one network call behind it.
//! A key typed into the card is redeemed at Pay through the site's Worker
//! (`licence::REDEEM_URL`); the certificate that comes back is kept with the
//! preferences and proves the licence offline. Once per run, a stored key
//! whose certificate is missing or ends within `licence::RENEW_WITHIN_SECONDS`
//! is offered to Pay again, which renews the certificate while the
//! subscription runs and refuses it once it has ended. Nothing here stops the
//! app: the card only says what the licence is. The first time the app runs
//! it asks once whether it is for personal or commercial use, or to try it
//! for work free for a day (`licence_prompt`, `LicenceUse::Trial`); the
//! answer is kept with the preferences, and a trial that has ended asks once
//! more.

use super::*;
use crate::licence::{self, Redeemed, Standing};

/// What a click on the card asks for, taken after the card is drawn.
enum LicenceAction {
    Activate(String),
    Retry,
    Remove,
    Open(&'static str),
    /// The first-run answer changed to personal use.
    Personal,
}

impl Desktop {
    pub(super) fn licence_card(&mut self, ui: &mut egui::Ui, family: &FontFamily) {
        let mut open = !self.collapsed[6];
        let now = vector_rebuild::clock::unix_seconds();
        let standing = licence::standing(&self.licence, now);
        let commercial_without_key = matches!(standing, Standing::Personal)
            && matches!(
                self.licence_use,
                Some(LicenceUse::Commercial | LicenceUse::Trial(_))
            );
        let trial = self.licence_use.and_then(|u| u.trial_left(now));
        let summary = match &standing {
            Standing::Personal if trial == Some(0) => {
                "Commercial trial ended \u{00B7} add your key".to_owned()
            }
            Standing::Personal if trial.is_some() => format!(
                "Commercial trial \u{00B7} {} left",
                time_left(trial.unwrap_or(0))
            ),
            Standing::Personal if commercial_without_key => {
                "Commercial \u{00B7} add your key".to_owned()
            }
            Standing::Personal => "Personal use".to_owned(),
            Standing::Commercial(verified) => format!(
                "Commercial \u{00B7} {} seat{}",
                verified.seats,
                if verified.seats == 1 { "" } else { "s" }
            ),
            Standing::Unproven => "Commercial \u{00B7} not confirmed".to_owned(),
        };
        let pending = self.licence_reply.is_some();
        let on_sale = licence::PRODUCT_ID.is_some();
        let mut action: Option<LicenceAction> = None;
        let button = |text: &str| {
            egui::Button::new(RichText::new(text).size(12.)).corner_radius(CornerRadius::same(10))
        };
        let note = |ui: &mut egui::Ui, text: &str, color: Color32| {
            ui.add(egui::Label::new(RichText::new(text).size(11.5).color(color)).wrap());
        };
        card(
            ui,
            family,
            icon::FILE,
            "License",
            &mut open,
            &summary,
            true,
            |ui| {
                match &standing {
                    Standing::Personal if trial == Some(0) => {
                        note(
                            ui,
                            &format!(
                                "Your free day of commercial use has ended. For work, a \
                                 commercial license is {}: buy one, then paste the key \
                                 from its email here.",
                                licence::PRICE
                            ),
                            pal().dim,
                        );
                    }
                    Standing::Personal if trial.is_some() => {
                        note(
                            ui,
                            &format!(
                                "Your free commercial trial ends in {}. To keep using \
                                 VectorMagik for work after that, a commercial license \
                                 is {}: buy one, then paste the key from its email here.",
                                time_left(trial.unwrap_or(0)),
                                licence::PRICE
                            ),
                            pal().dim,
                        );
                    }
                    Standing::Personal if commercial_without_key => {
                        note(
                            ui,
                            &format!(
                                "You use VectorMagik for work: a commercial license is \
                                 {}. Buy one, then paste the key from its email here.",
                                licence::PRICE
                            ),
                            pal().dim,
                        );
                    }
                    Standing::Personal => {
                        note(
                            ui,
                            &format!(
                                "Free for personal and other noncommercial use. Using \
                                 VectorMagik for work? A commercial license is {}.",
                                licence::PRICE
                            ),
                            pal().dim,
                        );
                    }
                    _ => {}
                }
                if matches!(standing, Standing::Personal) {
                    if !on_sale {
                        note(ui, "Commercial licenses go on sale soon.", pal().faint);
                    }
                    let (label, url) = if on_sale {
                        ("Buy a license", licence::BUY_URL)
                    } else {
                        ("License details", licence::LICENCE_PAGE)
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(6., 6.);
                        if ui.add(button(label)).on_hover_text(url).clicked() {
                            action = Some(LicenceAction::Open(url));
                        }
                        if commercial_without_key
                            && ui
                                .add(button("Personal use only"))
                                .on_hover_text("Not for work after all: VectorMagik stays free.")
                                .clicked()
                        {
                            action = Some(LicenceAction::Personal);
                        }
                    });
                    if on_sale {
                        ui.add_space(2.);
                        note(ui, "Bought one? Paste your license key:", pal().dim);
                        let key = licence::normalize_key(&self.licence_input);
                        ui.horizontal(|ui| {
                            let field = ui.add(
                                egui::TextEdit::singleline(&mut self.licence_input)
                                    .hint_text("esk_XXXXX-XXXXX-XXXXX-XXXXX")
                                    .desired_width(ui.available_width() - 76.),
                            );
                            let ready = key.is_some() && !pending;
                            let enter =
                                field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if ui
                                .add_enabled(ready, button("Activate"))
                                .on_hover_text(
                                    "Checks the key with the license server once; after \
                                         that the license is proved offline.",
                                )
                                .on_disabled_hover_text(if pending {
                                    "Waiting for the license server."
                                } else {
                                    "Paste a key that starts with esk_."
                                })
                                .clicked()
                                || (ready && enter)
                            {
                                if let Some(key) = key {
                                    action = Some(LicenceAction::Activate(key));
                                }
                            }
                        });
                    }
                }
                match &standing {
                    Standing::Personal => {}
                    Standing::Commercial(verified) => {
                        note(
                            ui,
                            &format!(
                                "Commercial license, {} seat{}. Key {} \u{00B7} license {}.",
                                verified.seats,
                                if verified.seats == 1 { "" } else { "s" },
                                licence::key_hint(&self.licence.key),
                                &verified.licence_id[..verified.licence_id.len().min(8)],
                            ),
                            pal().text,
                        );
                        note(
                            ui,
                            &format!(
                                "Confirmed until {}; it renews by itself while the \
                                 subscription runs.",
                                licence::date(verified.expires)
                            ),
                            pal().faint,
                        );
                    }
                    Standing::Unproven => {
                        note(
                            ui,
                            &format!(
                                "Key {} is waiting for the license server to confirm it.",
                                licence::key_hint(&self.licence.key)
                            ),
                            pal().dim,
                        );
                        if ui.add_enabled(!pending, button("Try again")).clicked() {
                            action = Some(LicenceAction::Retry);
                        }
                    }
                }
                if !matches!(standing, Standing::Personal) {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(6., 6.);
                        if ui
                            .add(button("Manage subscription"))
                            .on_hover_text("Change seats, update the card or cancel.")
                            .clicked()
                        {
                            action = Some(LicenceAction::Open(licence::MANAGE_URL));
                        }
                        if ui
                            .add(button("Remove"))
                            .on_hover_text(
                                "Forgets the key here. The license itself is untouched: \
                                 paste the key again to bring it back.",
                            )
                            .clicked()
                        {
                            action = Some(LicenceAction::Remove);
                        }
                    });
                }
                if pending {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.).color(pal().accent));
                        note(ui, "Asking the license server\u{2026}", pal().dim);
                    });
                }
                if let Some(text) = &self.licence_note {
                    note(ui, text, pal().warn);
                }
            },
        );
        self.collapsed[6] = !open;
        let ctx = ui.ctx().clone();
        match action {
            Some(LicenceAction::Activate(key)) => self.start_redeem(&ctx, key, true),
            Some(LicenceAction::Retry) => {
                let key = self.licence.key.clone();
                self.start_redeem(&ctx, key, false);
            }
            Some(LicenceAction::Remove) => {
                self.licence = licence::Stored::default();
                self.licence_note = None;
            }
            Some(LicenceAction::Personal) => self.licence_use = Some(LicenceUse::Personal),
            Some(LicenceAction::Open(url)) => self.open_licence_url(&ctx, url),
            None => {}
        }
    }

    /// The first-run question: personal or commercial use, asked once when
    /// the preferences hold no answer and no licence (`ask_licence_if_new`).
    /// Personal closes it; Commercial opens a key field, a Buy button and a
    /// way on without a key; a key activated here answers it too.
    pub(super) fn licence_prompt(&mut self, ctx: &egui::Context) {
        if !self.ask_licence {
            return;
        }
        let family = self.title_family.clone();
        let pending = self.licence_reply.is_some();
        let p = pal();
        let mut answer: Option<LicenceUse> = None;
        let mut action: Option<LicenceAction> = None;
        let logo = self.logo.as_ref().map(|logo| logo.id());
        let key = licence::normalize_key(&self.licence_input);
        let width = dialog_width(ctx, 500.);
        let opened = self.licence_opened;
        let now = vector_rebuild::clock::unix_seconds();
        // Asked again because the free day is over: no second trial.
        let trial_ended = self.licence_use.and_then(|u| u.trial_left(now)) == Some(0);
        egui::Modal::new(egui::Id::new("licence-prompt"))
            .frame(popup_frame().inner_margin(Margin::same(24)))
            .backdrop_color(p.scrim)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 10.;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.;
                    if let Some(logo) = logo {
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(
                            logo,
                            Vec2::splat(44.),
                        )));
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.;
                        ui.label(
                            RichText::new("Welcome to VectorMagik")
                                .size(21.)
                                .family(family.clone())
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(if trial_ended {
                                "Your free day of commercial use has ended. How will you \
                                 use it now?"
                            } else {
                                "How will you use it?"
                            })
                            .size(13.5)
                            .color(p.dim),
                        );
                    });
                });
                ui.add_space(2.);
                let (personal, commercial) = tile_pair(
                    ui,
                    &family,
                    width < 440.,
                    (
                        [
                            "Personal",
                            "Free",
                            "For yourself: hobbies, study and other noncommercial use.",
                            "Use it free",
                        ],
                        false,
                    ),
                    (
                        [
                            "Commercial",
                            licence::PRICE,
                            "For work: client jobs, products and anything you sell.",
                            "Choose commercial",
                        ],
                        self.prompt_key,
                    ),
                );
                if personal {
                    answer = Some(LicenceUse::Personal);
                }
                if commercial {
                    self.prompt_key = true;
                }
                if !trial_ended
                    && ui
                        .add(pill_widget(
                            "Not sure yet? Try it for work free for 24 hours",
                        ))
                        .on_hover_text(
                            "Commercial use free for a day: no key and no payment. After \
                             that, the app asks again.",
                        )
                        .clicked()
                {
                    answer = Some(LicenceUse::Trial(now));
                }
                reveal(ui, "licence-key", self.prompt_key, |ui| {
                    ui.add_space(2.);
                    ui.label(
                        RichText::new("Paste the license key from your purchase email:")
                            .size(12.5)
                            .color(p.dim),
                    );
                    ui.horizontal(|ui| {
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut self.licence_input)
                                .hint_text("esk_XXXXX-XXXXX-XXXXX-XXXXX")
                                .margin(Margin::symmetric(10, 7))
                                .desired_width(ui.available_width() - 96.),
                        );
                        let ready = key.is_some() && !pending;
                        let enter =
                            field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if ui
                            .add_enabled(
                                ready,
                                egui::Button::new(RichText::new("Activate").color(p.on_accent))
                                    .fill(p.accent)
                                    .corner_radius(CornerRadius::same(p.control_radius.max(8)))
                                    .min_size(Vec2::new(88., 30.)),
                            )
                            .on_disabled_hover_text(if pending {
                                "Waiting for the license server."
                            } else {
                                "Paste a key that starts with esk_."
                            })
                            .clicked()
                            || (ready && enter)
                        {
                            if let Some(key) = key.clone() {
                                action = Some(LicenceAction::Activate(key));
                            }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(8., 6.);
                        if ui
                            .add(pill_widget("Buy a license"))
                            .on_hover_text(format!("{}: opens the checkout", licence::PRICE))
                            .clicked()
                        {
                            action = Some(LicenceAction::Open(licence::BUY_URL));
                        }
                        if ui
                            .add(pill_widget("Continue without a key"))
                            .on_hover_text(
                                "Paste the key later in the License card at the bottom of \
                                 the left panel.",
                            )
                            .clicked()
                        {
                            answer = Some(LicenceUse::Commercial);
                        }
                    });
                    if opened {
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    "The checkout opened in a new tab of your browser. After \
                                     paying, paste the key from its email above.",
                                )
                                .size(11.5)
                                .color(p.dim),
                            )
                            .wrap(),
                        );
                    }
                    if pending {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(12.).color(p.accent));
                            ui.label(
                                RichText::new("Asking the license server\u{2026}")
                                    .size(11.5)
                                    .color(p.dim),
                            );
                        });
                    }
                    if let Some(text) = &self.licence_note {
                        ui.add(
                            egui::Label::new(RichText::new(text).size(11.5).color(p.warn)).wrap(),
                        );
                    }
                });
                ui.label(
                    RichText::new(
                        "You can change this later in the License card at the bottom of \
                         the left panel.",
                    )
                    .size(11.)
                    .color(p.faint),
                );
            });
        if let Some(answer) = answer {
            self.licence_use = Some(answer);
            self.ask_licence = false;
            self.prompt_key = false;
        }
        match action {
            Some(LicenceAction::Activate(key)) => self.start_redeem(ctx, key, true),
            Some(LicenceAction::Open(url)) => self.open_licence_url(ctx, url),
            _ => {}
        }
    }

    /// Open one of the licence's pages in the browser and say so: a new tab
    /// can open behind the window or out of sight, and the five simulated
    /// visitors of September 24, 2026 read a click that seemed to do nothing
    /// as broken.
    fn open_licence_url(&mut self, ctx: &egui::Context, url: &str) {
        platform::open_url(ctx, url);
        let what = if url == licence::BUY_URL {
            self.licence_opened = true;
            "the checkout: buy there, then paste the key from its email here"
        } else if url == licence::MANAGE_URL {
            "your subscription's page"
        } else {
            "the license page"
        };
        self.set_status(
            StatusKind::Info,
            format!("Opened {what} in your web browser."),
        );
    }

    /// Present `key` to Pay; `typed` when it came from the card's field.
    pub(super) fn start_redeem(&mut self, ctx: &egui::Context, key: String, typed: bool) {
        if key.is_empty() || self.licence_reply.is_some() {
            return;
        }
        let reply = platform::post_json(ctx, licence::REDEEM_URL, licence::redeem_body(&key));
        self.licence_reply = Some((reply, key, typed));
        self.licence_note = None;
    }

    /// Once a frame: take a redeem's reply when it has come, and once per run
    /// offer the stored key for renewal when its certificate wants it.
    pub(super) fn licence_tick(&mut self, ctx: &egui::Context) {
        if let Some((reply, _, _)) = &self.licence_reply {
            match reply.try_recv() {
                Ok((status, body)) => {
                    if let Some((_, key, typed)) = self.licence_reply.take() {
                        self.take_redeem(key, typed, status, &body);
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.licence_reply = None;
                    self.licence_note = Some("The license check stopped; try again.".into());
                }
            }
            return;
        }
        if !self.licence_renewed {
            self.licence_renewed = true;
            let now = vector_rebuild::clock::unix_seconds();
            if licence::wants_redeem(&self.licence, now) {
                let key = self.licence.key.clone();
                self.start_redeem(ctx, key, false);
            }
        }
    }

    /// Act on a redeem's reply for `key`.
    pub(super) fn take_redeem(&mut self, key: String, typed: bool, status: u16, body: &str) {
        let now = vector_rebuild::clock::unix_seconds();
        match licence::read_redeem(status, body) {
            Redeemed::Certificate(certificate) => {
                let Some(product) = licence::PRODUCT_ID else {
                    return;
                };
                match licence::verify(&certificate, product, &licence::subject(&key), now) {
                    Ok(verified) => {
                        self.licence = licence::Stored { key, certificate };
                        self.licence_input.clear();
                        self.licence_note = None;
                        // A key that works answers the first-run question.
                        self.licence_use = Some(LicenceUse::Commercial);
                        self.ask_licence = false;
                        self.prompt_key = false;
                        self.set_status(
                            StatusKind::Done,
                            format!(
                                "Commercial license confirmed until {}.",
                                licence::date(verified.expires)
                            ),
                        );
                    }
                    Err(_) => {
                        self.licence_note = Some(
                            "The license server sent a certificate this app cannot use \
                             (another product's, or an update is needed)."
                                .into(),
                        );
                    }
                }
            }
            Redeemed::Refused(reason) if typed => self.licence_note = Some(reason),
            Redeemed::Refused(reason) => {
                // The stored key itself was refused: its licence has ended.
                self.licence = licence::Stored::default();
                self.licence_note = Some(format!("The license has ended: {reason}"));
            }
            Redeemed::Unreachable(reason) => self.licence_note = Some(reason),
        }
    }
}

/// A trial's time left in words: whole hours, rounded up, or minutes in its
/// last hour.
fn time_left(seconds: i64) -> String {
    let (count, unit) = if seconds >= 3600 {
        ((seconds + 3599) / 3600, "hour")
    } else {
        (((seconds + 59) / 60).max(1), "minute")
    };
    format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
}
