//! The objects a PostScript program works with, shared the way PostScript
//! shares them: a string, array or dictionary is one value however many
//! places hold it, and `getinterval` is a window onto the same storage.
//!
//! Each string, array and dictionary is charged against a count of the
//! bytes alive on this thread, so a program cannot hold more memory than an
//! import may use, and arrays and dictionaries are dropped without
//! recursion, so a hostile chain of a million nested arrays cannot overflow
//! the stack when it goes.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use super::stream::File;

thread_local! {
    static LIVE: Cell<usize> = const { Cell::new(0) };
}

/// The bytes the strings, arrays and dictionaries alive on this thread hold.
pub(super) fn live() -> usize {
    LIVE.with(Cell::get)
}

/// What one string, array or dictionary is charged, given back when it is
/// dropped.
struct Meter(Cell<usize>);

impl Meter {
    fn new(bytes: usize) -> Self {
        LIVE.with(|live| live.set(live.get() + bytes));
        Self(Cell::new(bytes))
    }

    fn grow(&self, bytes: usize) {
        LIVE.with(|live| live.set(live.get() + bytes));
        self.0.set(self.0.get() + bytes);
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        let bytes = self.0.get();
        LIVE.with(|live| live.set(live.get().saturating_sub(bytes)));
    }
}

/// A name: its bytes, compared by value.
pub(super) type Name = Rc<[u8]>;

/// One PostScript object. Names, strings, arrays and files carry the
/// executable attribute; the access attributes are not kept, except that
/// a dictionary can be read-only (systemdict is).
#[derive(Clone)]
pub(super) enum Obj {
    Null,
    Int(i32),
    Real(f64),
    Bool(bool),
    Name(Name, bool),
    Str(Str),
    Array(Arr),
    Dict(Dict),
    /// An operator, by its place in `ops::OPS`.
    Op(u16),
    Mark,
    File(File, bool),
    Save(u32),
    /// The identifier `definefont` gives a font.
    FontId,
}

impl Obj {
    /// A literal name.
    pub(super) fn name(text: &str) -> Obj {
        Obj::Name(text.as_bytes().into(), false)
    }

    /// An executable name.
    pub(super) fn command(text: &str) -> Obj {
        Obj::Name(text.as_bytes().into(), true)
    }

    pub(super) fn string(bytes: &[u8]) -> Obj {
        Obj::Str(Str::new(bytes.to_vec()))
    }

    /// An empty procedure.
    pub(super) fn nothing() -> Obj {
        Obj::Array(Arr::new(Vec::new(), true))
    }

    /// A literal array of numbers.
    pub(super) fn numbers(values: &[f64]) -> Obj {
        Obj::Array(Arr::new(
            values.iter().map(|v| Obj::Real(*v)).collect(),
            false,
        ))
    }

    pub(super) fn num(&self) -> Option<f64> {
        match self {
            Obj::Int(i) => Some(f64::from(*i)),
            Obj::Real(r) => Some(*r),
            _ => None,
        }
    }

    pub(super) fn executable(&self) -> bool {
        match self {
            Obj::Name(_, exec) | Obj::File(_, exec) => *exec,
            Obj::Str(s) => s.exec,
            Obj::Array(a) => a.exec,
            Obj::Op(_) => true,
            _ => false,
        }
    }

    /// The object with its executable attribute set to `exec`.
    pub(super) fn with_exec(self, exec: bool) -> Obj {
        match self {
            Obj::Name(name, _) => Obj::Name(name, exec),
            Obj::File(file, _) => Obj::File(file, exec),
            Obj::Str(s) => Obj::Str(Str { exec, ..s }),
            Obj::Array(a) => Obj::Array(Arr { exec, ..a }),
            other => other,
        }
    }

    /// The name `type` answers for the object.
    pub(super) fn type_name(&self) -> &'static str {
        match self {
            Obj::Null => "nulltype",
            Obj::Int(_) => "integertype",
            Obj::Real(_) => "realtype",
            Obj::Bool(_) => "booleantype",
            Obj::Name(..) => "nametype",
            Obj::Str(_) => "stringtype",
            Obj::Array(_) => "arraytype",
            Obj::Dict(_) => "dicttype",
            Obj::Op(_) => "operatortype",
            Obj::Mark => "marktype",
            Obj::File(..) => "filetype",
            Obj::Save(_) => "savetype",
            Obj::FontId => "fonttype",
        }
    }

    /// The bytes of a name or a string.
    pub(super) fn text(&self) -> Option<Vec<u8>> {
        match self {
            Obj::Name(name, _) => Some(name.to_vec()),
            Obj::Str(s) => Some(s.to_vec()),
            _ => None,
        }
    }
}

/// Whether two objects are equal as `eq` tells: numbers by value, strings
/// and names by their bytes, everything else by identity.
pub(super) fn same(a: &Obj, b: &Obj) -> bool {
    match (a, b) {
        (Obj::Int(x), Obj::Int(y)) => x == y,
        (Obj::Int(_) | Obj::Real(_), Obj::Int(_) | Obj::Real(_)) => a.num() == b.num(),
        (Obj::Bool(x), Obj::Bool(y)) => x == y,
        (Obj::Null, Obj::Null) | (Obj::Mark, Obj::Mark) | (Obj::FontId, Obj::FontId) => true,
        (Obj::Name(x, _), Obj::Name(y, _)) => x == y,
        (Obj::Name(..) | Obj::Str(_), Obj::Name(..) | Obj::Str(_)) => a.text() == b.text(),
        (Obj::Array(x), Obj::Array(y)) => x.same(y),
        (Obj::Dict(x), Obj::Dict(y)) => x.same(y),
        (Obj::Op(x), Obj::Op(y)) => x == y,
        (Obj::File(x, _), Obj::File(y, _)) => Rc::ptr_eq(x, y),
        (Obj::Save(x), Obj::Save(y)) => x == y,
        _ => false,
    }
}

/// A string: a window onto shared bytes.
#[derive(Clone)]
pub(super) struct Str {
    body: Rc<StrBody>,
    start: usize,
    pub(super) len: usize,
    pub(super) exec: bool,
}

struct StrBody {
    bytes: RefCell<Vec<u8>>,
    _meter: Meter,
}

impl Str {
    pub(super) fn new(bytes: Vec<u8>) -> Self {
        let len = bytes.len();
        Self {
            body: Rc::new(StrBody {
                _meter: Meter::new(len + 48),
                bytes: RefCell::new(bytes),
            }),
            start: 0,
            len,
            exec: false,
        }
    }

    pub(super) fn to_vec(&self) -> Vec<u8> {
        self.body.bytes.borrow()[self.start..self.start + self.len].to_vec()
    }

    pub(super) fn get(&self, index: usize) -> u8 {
        self.body.bytes.borrow()[self.start + index]
    }

    pub(super) fn set(&self, index: usize, value: u8) {
        self.body.bytes.borrow_mut()[self.start + index] = value;
    }

    /// Writes `data` at `at`; the caller has checked that it fits.
    pub(super) fn write(&self, at: usize, data: &[u8]) {
        let from = self.start + at;
        self.body.bytes.borrow_mut()[from..from + data.len()].copy_from_slice(data);
    }

    pub(super) fn interval(&self, start: usize, len: usize) -> Str {
        Str {
            body: self.body.clone(),
            start: self.start + start,
            len,
            exec: self.exec,
        }
    }
}

/// An array or a procedure: a window onto shared elements.
#[derive(Clone)]
pub(super) struct Arr {
    body: Rc<ArrBody>,
    start: usize,
    pub(super) len: usize,
    pub(super) exec: bool,
}

struct ArrBody {
    items: RefCell<Vec<Obj>>,
    _meter: Meter,
}

impl Drop for ArrBody {
    fn drop(&mut self) {
        let items = std::mem::take(self.items.get_mut());
        if !items.is_empty() {
            drain(items);
        }
    }
}

impl Arr {
    pub(super) fn new(items: Vec<Obj>, exec: bool) -> Self {
        let len = items.len();
        Self {
            body: Rc::new(ArrBody {
                _meter: Meter::new(len * std::mem::size_of::<Obj>() + 48),
                items: RefCell::new(items),
            }),
            start: 0,
            len,
            exec,
        }
    }

    pub(super) fn get(&self, index: usize) -> Obj {
        self.body.items.borrow()[self.start + index].clone()
    }

    pub(super) fn set(&self, index: usize, value: Obj) {
        self.body.items.borrow_mut()[self.start + index] = value;
    }

    pub(super) fn to_vec(&self) -> Vec<Obj> {
        self.body.items.borrow()[self.start..self.start + self.len].to_vec()
    }

    pub(super) fn interval(&self, start: usize, len: usize) -> Arr {
        Arr {
            body: self.body.clone(),
            start: self.start + start,
            len,
            exec: self.exec,
        }
    }

    /// Where the array's elements live, to tell arrays apart.
    pub(super) fn id(&self) -> (usize, usize) {
        (Rc::as_ptr(&self.body) as usize, self.start)
    }

    fn same(&self, other: &Arr) -> bool {
        Rc::ptr_eq(&self.body, &other.body) && self.start == other.start && self.len == other.len
    }

    /// The array's numbers, or `None` when an element is not a number.
    pub(super) fn numbers(&self) -> Option<Vec<f64>> {
        self.body.items.borrow()[self.start..self.start + self.len]
            .iter()
            .map(Obj::num)
            .collect()
    }
}

/// A dictionary: keys in the order they were first defined, so that
/// `forall` visits them the same way on every run.
#[derive(Clone)]
pub(super) struct Dict(Rc<DictBody>);

struct DictBody {
    table: RefCell<Table>,
    readonly: Cell<bool>,
    capacity: Cell<usize>,
    meter: Meter,
}

impl Drop for DictBody {
    fn drop(&mut self) {
        let table = std::mem::take(self.table.get_mut());
        let mut pending = Vec::with_capacity(table.entries.len() * 2);
        for (key, value) in table.entries {
            pending.push(key);
            pending.push(value);
        }
        if !pending.is_empty() {
            drain(pending);
        }
    }
}

#[derive(Default)]
struct Table {
    entries: Vec<(Obj, Obj)>,
    names: HashMap<Name, usize>,
    others: HashMap<Key, usize>,
}

/// A key that is not a name, by value or identity.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Int(i64),
    Real(u64),
    Bool(bool),
    Op(u16),
    Mark,
    Save(u32),
    Font,
    Ref(usize, usize, usize),
}

/// Where a key is filed, and the key as it is kept (strings become names,
/// whole reals integers).
enum Slot {
    Name(Name),
    Other(Key),
}

fn slot(key: &Obj) -> Result<(Slot, Obj), &'static str> {
    Ok(match key {
        Obj::Name(name, _) => (Slot::Name(name.clone()), Obj::Name(name.clone(), false)),
        Obj::Str(s) => {
            let name: Name = s.to_vec().into();
            (Slot::Name(name.clone()), Obj::Name(name, false))
        }
        Obj::Int(i) => (Slot::Other(Key::Int(i64::from(*i))), key.clone()),
        Obj::Real(r) if r.fract() == 0. && r.abs() < 9e15 => {
            (Slot::Other(Key::Int(*r as i64)), Obj::Int(*r as i32))
        }
        Obj::Real(r) => (Slot::Other(Key::Real(r.to_bits())), key.clone()),
        Obj::Bool(b) => (Slot::Other(Key::Bool(*b)), key.clone()),
        Obj::Op(op) => (Slot::Other(Key::Op(*op)), key.clone()),
        Obj::Mark => (Slot::Other(Key::Mark), key.clone()),
        Obj::Save(s) => (Slot::Other(Key::Save(*s)), key.clone()),
        Obj::FontId => (Slot::Other(Key::Font), key.clone()),
        Obj::Array(a) => {
            let (body, start) = a.id();
            (Slot::Other(Key::Ref(body, start, a.len)), key.clone())
        }
        Obj::Dict(d) => (
            Slot::Other(Key::Ref(Rc::as_ptr(&d.0) as usize, 0, 0)),
            key.clone(),
        ),
        Obj::File(f, _) => (
            Slot::Other(Key::Ref(Rc::as_ptr(f) as *const u8 as usize, 0, 0)),
            key.clone(),
        ),
        Obj::Null => return Err("typecheck"),
    })
}

impl Table {
    fn position(&self, slot: &Slot) -> Option<usize> {
        match slot {
            Slot::Name(name) => self.names.get(name).copied(),
            Slot::Other(key) => self.others.get(key).copied(),
        }
    }
}

impl Dict {
    pub(super) fn new(capacity: usize) -> Self {
        Self(Rc::new(DictBody {
            table: RefCell::new(Table::default()),
            readonly: Cell::new(false),
            capacity: Cell::new(capacity.min(1 << 16)),
            meter: Meter::new(96),
        }))
    }

    pub(super) fn size(&self) -> usize {
        self.0.table.borrow().entries.len()
    }

    /// What `maxlength` answers: the capacity asked for, or more once it
    /// has grown past it.
    pub(super) fn capacity(&self) -> usize {
        self.0.capacity.get().max(self.size())
    }

    pub(super) fn get(&self, key: &Obj) -> Option<Obj> {
        let (slot, _) = slot(key).ok()?;
        let table = self.0.table.borrow();
        table.position(&slot).map(|at| table.entries[at].1.clone())
    }

    /// The value of a name, looked up without making the name.
    pub(super) fn find(&self, name: &[u8]) -> Option<Obj> {
        let table = self.0.table.borrow();
        table.names.get(name).map(|at| table.entries[*at].1.clone())
    }

    pub(super) fn put(&self, key: Obj, value: Obj) -> Result<(), &'static str> {
        if self.0.readonly.get() {
            return Err("invalidaccess");
        }
        let (slot, key) = slot(&key)?;
        let mut table = self.0.table.borrow_mut();
        match table.position(&slot) {
            Some(at) => table.entries[at].1 = value,
            None => {
                let at = table.entries.len();
                table.entries.push((key, value));
                match slot {
                    Slot::Name(name) => table.names.insert(name, at),
                    Slot::Other(key) => table.others.insert(key, at),
                };
                self.0.meter.grow(2 * std::mem::size_of::<Obj>() + 32);
            }
        }
        Ok(())
    }

    /// Defines a name while the interpreter sets itself up (read-only
    /// dictionaries included).
    pub(super) fn set(&self, name: &str, value: Obj) {
        let readonly = self.0.readonly.replace(false);
        let _ = self.put(Obj::name(name), value);
        self.0.readonly.set(readonly);
    }

    pub(super) fn remove(&self, key: &Obj) -> Result<(), &'static str> {
        if self.0.readonly.get() {
            return Err("invalidaccess");
        }
        let (place, _) = slot(key)?;
        let mut table = self.0.table.borrow_mut();
        let Some(at) = table.position(&place) else {
            return Ok(());
        };
        match place {
            Slot::Name(name) => table.names.remove(&name),
            Slot::Other(key) => table.others.remove(&key),
        };
        table.entries.swap_remove(at);
        if at < table.entries.len() {
            // The last entry moved into the gap: file it at its new place.
            if let Ok((moved, _)) = slot(&table.entries[at].0) {
                match moved {
                    Slot::Name(name) => table.names.insert(name, at),
                    Slot::Other(key) => table.others.insert(key, at),
                };
            }
        }
        Ok(())
    }

    /// The entries in the order they were defined.
    pub(super) fn entries(&self) -> Vec<(Obj, Obj)> {
        self.0.table.borrow().entries.clone()
    }

    pub(super) fn same(&self, other: &Dict) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub(super) fn readonly(&self) -> bool {
        self.0.readonly.get()
    }

    pub(super) fn make_readonly(&self) {
        self.0.readonly.set(true);
    }
}

/// Drops objects without recursion: each array or dictionary that is the
/// last holder of its contents hands them to the list instead of dropping
/// them itself.
fn drain(mut pending: Vec<Obj>) {
    while let Some(obj) = pending.pop() {
        match obj {
            Obj::Array(Arr { body, .. }) => {
                if let Ok(body) = Rc::try_unwrap(body) {
                    pending.extend(body.items.take());
                }
            }
            Obj::Dict(Dict(body)) => {
                if let Ok(body) = Rc::try_unwrap(body) {
                    for (key, value) in body.table.take().entries {
                        pending.push(key);
                        pending.push(value);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Empties every array and dictionary reachable from `roots`, which frees
/// the cycles a program may have built (an array holding itself) once the
/// interpreter is done.
pub(super) fn dismantle(mut pending: Vec<Obj>) {
    while let Some(obj) = pending.pop() {
        match &obj {
            Obj::Array(a) => pending.extend(a.body.items.take()),
            Obj::Dict(d) => {
                for (key, value) in d.0.table.take().entries {
                    pending.push(key);
                    pending.push(value);
                }
            }
            _ => {}
        }
    }
}
