//! Photoshop's action descriptors: the typed trees of keys and values in
//! which a layer keeps its fill (`SoCo`, `GdFl`, `PtFl`, `vscg`), its vector
//! stroke (`vstk`) and its effects (`lfx2`). This follows Adobe's file format
//! specification and reads every value type it lists, plus `UnFl` and
//! `ObAr`, which it leaves out but Photoshop writes; the values shape layers
//! need are kept and the rest (references, classes, aliases, raw data) are
//! read past. Descriptors nest at most `DEPTH` deep and every length and
//! count is checked against the bytes that are left, so damaged or hostile
//! data is refused with an error instead of being read past or allocated
//! for.

use super::Reader;

/// What a descriptor this reader cannot follow says.
pub(super) const DAMAGED: &str = "A layer's settings in this Photoshop document are damaged";

/// How deep descriptors and lists may nest; Photoshop's own go five or six
/// deep.
const DEPTH: usize = 32;

/// A descriptor: its class and its items in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Descriptor {
    pub class: String,
    pub items: Vec<(String, Value)>,
}

/// One value of a descriptor or a list.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Object(Descriptor),
    List(Vec<Value>),
    Double(f64),
    /// A double with its unit (`#Pxl`, `#Pnt`, `#Prc`, `#Ang`...).
    Unit([u8; 4], f64),
    Integer(i64),
    Bool(bool),
    /// An enumerated value: its type, then the value.
    Enum(String, String),
    Text(String),
    /// A value of a type that shape layers do not need, read past.
    Other,
}

impl Descriptor {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn object(&self, key: &str) -> Option<&Descriptor> {
        match self.get(key)? {
            Value::Object(object) => Some(object),
            _ => None,
        }
    }

    pub fn list(&self, key: &str) -> Option<&[Value]> {
        match self.get(key)? {
            Value::List(list) => Some(list),
            _ => None,
        }
    }

    /// A number whatever its type: a double, a unit float or an integer.
    pub fn number(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            Value::Double(v) | Value::Unit(_, v) => Some(*v),
            Value::Integer(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn unit(&self, key: &str) -> Option<([u8; 4], f64)> {
        match self.get(key)? {
            Value::Unit(unit, v) => Some((*unit, *v)),
            Value::Double(v) => Some((*b"#Nne", *v)),
            _ => None,
        }
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Value::Bool(v) => Some(*v),
            _ => None,
        }
    }

    /// The value of an enumerated item, without its type.
    pub fn enumerated(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Enum(_, v) => Some(v),
            _ => None,
        }
    }
}

/// The descriptor in `data` after its four-byte version, which is 16, as the
/// additional layer information blocks store one.
pub(super) fn versioned(data: &[u8]) -> Result<Descriptor, String> {
    let mut r = Reader::new(data);
    if r.u32()? != 16 {
        return Err(DAMAGED.into());
    }
    descriptor(&mut r, 0)
}

fn descriptor(r: &mut Reader, depth: usize) -> Result<Descriptor, String> {
    if depth > DEPTH {
        return Err(DAMAGED.into());
    }
    unicode(r)?; // The class's name, for people.
    let class = key(r)?;
    // Every item takes at least twelve bytes, so a count past what is left
    // ends in an error before long; nothing is reserved for it.
    let count = r.u32()?;
    let mut items = Vec::new();
    for _ in 0..count {
        let key = key(r)?;
        let kind = r.array::<4>()?;
        items.push((key, value(r, &kind, depth)?));
    }
    Ok(Descriptor { class, items })
}

/// One value of type `kind`, inside descriptors or lists nested `depth` deep.
fn value(r: &mut Reader, kind: &[u8; 4], depth: usize) -> Result<Value, String> {
    Ok(match kind {
        b"Objc" | b"GlbO" => Value::Object(descriptor(r, depth + 1)?),
        b"VlLs" => {
            if depth >= DEPTH {
                return Err(DAMAGED.into());
            }
            let count = r.u32()?;
            let mut items = Vec::new();
            for _ in 0..count {
                let kind = r.array::<4>()?;
                items.push(value(r, &kind, depth + 1)?);
            }
            Value::List(items)
        }
        b"doub" => Value::Double(f64::from_be_bytes(r.array()?)),
        b"UntF" => {
            let unit = r.array()?;
            Value::Unit(unit, f64::from_be_bytes(r.array()?))
        }
        b"UnFl" => {
            r.skip(4)?; // The unit.
            let count = u64::from(r.u32()?);
            r.skip(count * 8)?;
            Value::Other
        }
        b"long" => Value::Integer(r.i32()?.into()),
        b"comp" => Value::Integer(i64::from_be_bytes(r.array()?)),
        b"bool" => Value::Bool(r.u8()? != 0),
        b"enum" => {
            let kind = key(r)?;
            Value::Enum(kind, key(r)?)
        }
        b"TEXT" => Value::Text(unicode(r)?),
        b"type" | b"GlbC" => {
            class(r)?;
            Value::Other
        }
        b"alis" | b"tdta" | b"Pth " => {
            r.section(false)?;
            Value::Other
        }
        b"obj " => {
            reference(r)?;
            Value::Other
        }
        // An object array: a count, then a descriptor's name, class and
        // items (unit float lists).
        b"ObAr" => {
            r.skip(4)?;
            descriptor(r, depth + 1)?;
            Value::Other
        }
        _ => return Err(DAMAGED.into()),
    })
}

/// A reference, read past: a count, then each item's form and its parts.
fn reference(r: &mut Reader) -> Result<(), String> {
    for _ in 0..r.u32()? {
        match &r.array::<4>()? {
            b"prop" => {
                class(r)?;
                key(r)?;
            }
            b"Clss" => class(r)?,
            b"Enmr" => {
                class(r)?;
                key(r)?;
                key(r)?;
            }
            b"rele" => {
                class(r)?;
                r.skip(4)?;
            }
            b"Idnt" | b"indx" => r.skip(4)?,
            b"name" => {
                class(r)?;
                unicode(r)?;
            }
            _ => return Err(DAMAGED.into()),
        }
    }
    Ok(())
}

/// A class, read past: its name, then its id.
fn class(r: &mut Reader) -> Result<(), String> {
    unicode(r)?;
    key(r).map(drop)
}

/// A key or class id: its length, or zero for a four-character code.
fn key(r: &mut Reader) -> Result<String, String> {
    let n = match r.u32()? {
        0 => 4,
        n => u64::from(n),
    };
    Ok(String::from_utf8_lossy(r.take(n)?).into_owned())
}

/// A string: its length in UTF-16 units, then the units.
fn unicode(r: &mut Reader) -> Result<String, String> {
    let n = u64::from(r.u32()?);
    let units: Vec<u16> = r
        .take(n * 2)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|unit| u16::from_be_bytes(*unit))
        .collect();
    Ok(String::from_utf16_lossy(&units)
        .trim_end_matches('\0')
        .to_owned())
}
