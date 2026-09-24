//! The language's operators: the operand stack, arithmetic, comparison,
//! control, types and conversions, strings, arrays and dictionaries, and
//! the table every operator is found in. Files, filters, resources and the
//! device are in `system.rs`, the graphics operators in `graphics.rs` and
//! `images.rs`.
use std::cmp::Ordering;
use std::collections::HashSet;

use super::object::{same, Arr, Dict, Obj, Str};
use super::scan::{format_real, number};
use super::stream::{self, Stream};
use super::{graphics, images, system};
use super::{Fault, Machine, Res, MAX_ARRAY, MAX_DICTS, MAX_STRING};

pub(super) type OpFn = fn(&mut Machine) -> Res;
/// An operator: its name, what it does, and how many operands it takes at
/// most (what goes back on the stack when it fails).
pub(super) type Entry = (&'static str, OpFn, u8);

/// errordict's standard handler for every error: stop.
pub(super) const ERROR: u16 = 0;
pub(super) const HANDLE_ERROR: u16 = 1;

pub(super) fn fail<T>(error: &'static str) -> Res<T> {
    Err(Fault::Error(error))
}

/// A count as a PostScript integer.
pub(super) fn int(n: usize) -> Obj {
    Obj::Int(i32::try_from(n).unwrap_or(i32::MAX))
}

/// The operator tables: the language's, the system's, the graphics
/// operators, and those of images and text. An operator is its place in
/// them all, one after another.
fn tables() -> [&'static [Entry]; 4] {
    [OPS, system::OPS, graphics::OPS, images::OPS]
}

/// Every operator.
pub(super) fn all() -> impl Iterator<Item = Entry> {
    tables().into_iter().flat_map(|table| table.iter().copied())
}

pub(super) fn entry(op: u16) -> Entry {
    let mut op = usize::from(op);
    for table in tables() {
        match table.get(op) {
            Some(entry) => return *entry,
            None => op -= table.len(),
        }
    }
    OPS[0]
}

/// The operator of a name (for the setup, and Illustrator's aliases).
pub(super) fn find(name: &str) -> u16 {
    all().position(|entry| entry.0 == name).unwrap_or(0) as u16
}

fn arith(m: &mut Machine, int: fn(i32, i32) -> Option<i32>, real: fn(f64, f64) -> f64) -> Res {
    let b = m.pop()?;
    let a = m.pop()?;
    if let (Obj::Int(x), Obj::Int(y)) = (&a, &b) {
        if let Some(value) = int(*x, *y) {
            return m.answer(Obj::Int(value));
        }
    }
    match (a.num(), b.num()) {
        (Some(x), Some(y)) => m.real(real(x, y)),
        _ => fail("typecheck"),
    }
}

fn int_arith(m: &mut Machine, op: fn(i32, i32) -> i32) -> Res {
    let b = m.pop_int()?;
    let a = m.pop_int()?;
    if b == 0 {
        return fail("undefinedresult");
    }
    m.answer(Obj::Int(op(a, b)))
}

fn unary(m: &mut Machine, int: fn(i32) -> Option<i32>, real: fn(f64) -> f64) -> Res {
    match m.pop()? {
        Obj::Int(value) => match int(value) {
            Some(result) => m.answer(Obj::Int(result)),
            None => m.real(real(f64::from(value))),
        },
        Obj::Real(value) => m.real(real(value)),
        _ => fail("typecheck"),
    }
}

fn rounding(m: &mut Machine, real: fn(f64) -> f64) -> Res {
    match m.pop()? {
        Obj::Int(value) => m.answer(Obj::Int(value)),
        Obj::Real(value) => m.real(real(value)),
        _ => fail("typecheck"),
    }
}

fn real_fn(m: &mut Machine, real: fn(f64) -> f64) -> Res {
    let value = m.pop_num()?;
    m.real(real(value))
}

fn positive_fn(m: &mut Machine, real: fn(f64) -> f64) -> Res {
    let value = m.pop_num()?;
    if value <= 0. {
        return fail("rangecheck");
    }
    m.real(real(value))
}

/// A number from a number or a string that spells one.
fn number_of(obj: Obj) -> Res<f64> {
    match obj {
        Obj::Int(_) | Obj::Real(_) => Ok(obj.num().unwrap_or(0.)),
        Obj::Str(text) => {
            let text = text.to_vec();
            let trimmed = String::from_utf8_lossy(&text);
            number(trimmed.trim().as_bytes())
                .and_then(|n| n.num())
                .ok_or(Fault::Error("syntaxerror"))
        }
        _ => fail("typecheck"),
    }
}

fn cvi(m: &mut Machine) -> Res {
    if let Some(Obj::Int(_)) = m.stack.last() {
        return Ok(());
    }
    let value = number_of(m.pop()?)?.trunc();
    if !(-2_147_483_648.0..=2_147_483_647.0).contains(&value) {
        return fail("rangecheck");
    }
    m.answer(Obj::Int(value as i32))
}

fn compare(m: &mut Machine, test: fn(Ordering) -> bool) -> Res {
    let b = m.pop()?;
    let a = m.pop()?;
    let order = match (&a, &b) {
        (Obj::Str(x), Obj::Str(y)) => x.to_vec().cmp(&y.to_vec()),
        _ => match (a.num(), b.num()) {
            (Some(x), Some(y)) => x.partial_cmp(&y).ok_or(Fault::Error("undefinedresult"))?,
            _ => return fail("typecheck"),
        },
    };
    m.answer(Obj::Bool(test(order)))
}

fn logic(m: &mut Machine, bools: fn(bool, bool) -> bool, ints: fn(i32, i32) -> i32) -> Res {
    let b = m.pop()?;
    let a = m.pop()?;
    match (a, b) {
        (Obj::Bool(x), Obj::Bool(y)) => m.answer(Obj::Bool(bools(x, y))),
        (Obj::Int(x), Obj::Int(y)) => m.answer(Obj::Int(ints(x, y))),
        _ => fail("typecheck"),
    }
}

fn copy(m: &mut Machine) -> Res {
    if let Some(&Obj::Int(n)) = m.stack.last() {
        let n = usize::try_from(n).map_err(|_| Fault::Error("rangecheck"))?;
        if n >= m.stack.len() {
            return fail("stackunderflow");
        }
        m.stack.pop();
        let from = m.stack.len() - n;
        m.stack.extend_from_within(from..);
        return Ok(());
    }
    let target = m.pop()?;
    let source = m.pop()?;
    match (source, target) {
        (Obj::Array(from), Obj::Array(to)) => {
            if from.len > to.len {
                return fail("rangecheck");
            }
            for (i, item) in from.to_vec().into_iter().enumerate() {
                to.set(i, item);
            }
            m.answer(Obj::Array(to.interval(0, from.len)))
        }
        (Obj::Str(from), Obj::Str(to)) => {
            if from.len > to.len {
                return fail("rangecheck");
            }
            to.write(0, &from.to_vec());
            m.answer(Obj::Str(to.interval(0, from.len)))
        }
        (Obj::Dict(from), Obj::Dict(to)) => {
            for (key, value) in from.entries() {
                to.put(key, value).map_err(Fault::Error)?;
            }
            m.answer(Obj::Dict(to))
        }
        _ => fail("typecheck"),
    }
}

fn index(m: &mut Machine) -> Res {
    let n = m.pop_int()?;
    let len = m.stack.len();
    let n = usize::try_from(n).map_err(|_| Fault::Error("rangecheck"))?;
    if n >= len {
        return fail("rangecheck");
    }
    let item = m.stack[len - 1 - n].clone();
    m.answer(item)
}

fn roll(m: &mut Machine) -> Res {
    let shift = m.pop_int()?;
    let n = m.pop_int()?;
    let n = usize::try_from(n).map_err(|_| Fault::Error("rangecheck"))?;
    if n > m.stack.len() {
        return fail("stackunderflow");
    }
    if n == 0 {
        return Ok(());
    }
    let start = m.stack.len() - n;
    let shift = i64::from(shift).rem_euclid(n as i64) as usize;
    m.stack[start..].rotate_right(shift);
    Ok(())
}

/// Runs one turn of a loop's procedure: false when it exits.
pub(super) fn iterate(m: &mut Machine, procedure: &Obj) -> Res<bool> {
    m.step()?;
    match m.exec(procedure.clone()) {
        Ok(()) => Ok(true),
        Err(Fault::Exit) => Ok(false),
        Err(fault) => Err(fault),
    }
}

fn op_for(m: &mut Machine) -> Res {
    let procedure = m.pop_proc()?;
    let limit = m.pop()?;
    let step = m.pop()?;
    let start = m.pop()?;
    if let (Obj::Int(start), Obj::Int(step), Obj::Int(limit)) = (&start, &step, &limit) {
        let (mut value, step, limit) = (i64::from(*start), i64::from(*step), i64::from(*limit));
        while (step >= 0 && value <= limit) || (step < 0 && value >= limit) {
            m.push(Obj::Int(value as i32));
            if !iterate(m, &procedure)? {
                break;
            }
            value += step;
        }
        return Ok(());
    }
    let (Some(mut value), Some(step), Some(limit)) = (start.num(), step.num(), limit.num()) else {
        return fail("typecheck");
    };
    while (step >= 0. && value <= limit) || (step < 0. && value >= limit) {
        m.push(Obj::Real(value));
        if !iterate(m, &procedure)? {
            break;
        }
        value += step;
    }
    Ok(())
}

fn forall(m: &mut Machine) -> Res {
    let procedure = m.pop_proc()?;
    match m.pop()? {
        Obj::Array(array) => {
            for i in 0..array.len {
                m.push(array.get(i));
                if !iterate(m, &procedure)? {
                    break;
                }
            }
        }
        Obj::Str(text) => {
            for i in 0..text.len {
                m.push(Obj::Int(i32::from(text.get(i))));
                if !iterate(m, &procedure)? {
                    break;
                }
            }
        }
        Obj::Dict(dict) => {
            for (key, value) in dict.entries() {
                m.push(key);
                m.push(value);
                if !iterate(m, &procedure)? {
                    break;
                }
            }
        }
        _ => return fail("typecheck"),
    }
    Ok(())
}

fn stopped(m: &mut Machine) -> Res {
    let procedure = m.pop()?;
    let stopped = match m.exec(procedure) {
        Ok(()) => false,
        Err(Fault::Stop) => true,
        Err(Fault::Exit) => match m.raise("invalidexit", Obj::command("exit")) {
            Ok(()) => false,
            Err(Fault::Stop) => true,
            Err(fault) => return Err(fault),
        },
        Err(fault) => return Err(fault),
    };
    m.answer(Obj::Bool(stopped))
}

/// What `cvs` writes for an object.
fn text_of(value: &Obj) -> Vec<u8> {
    match value {
        Obj::Int(i) => i.to_string().into_bytes(),
        Obj::Real(r) => format_real(*r).into_bytes(),
        Obj::Bool(b) => b.to_string().into_bytes(),
        Obj::Name(..) | Obj::Str(_) => value.text().unwrap_or_default(),
        Obj::Op(op) => entry(*op).0.trim_start_matches('.').as_bytes().to_vec(),
        _ => b"--nostringval--".to_vec(),
    }
}

/// Writes `text` into the start of `target` and answers that part.
fn answer_text(m: &mut Machine, target: Str, text: &[u8]) -> Res {
    if text.len() > target.len {
        return fail("rangecheck");
    }
    target.write(0, text);
    m.answer(Obj::Str(target.interval(0, text.len())))
}

fn cvs(m: &mut Machine) -> Res {
    let target = m.pop_str()?;
    let value = m.pop()?;
    answer_text(m, target, &text_of(&value))
}

fn cvrs(m: &mut Machine) -> Res {
    let target = m.pop_str()?;
    let radix = m.pop_int()?;
    let value = m.pop()?;
    if !(2..=36).contains(&radix) {
        return fail("rangecheck");
    }
    let text = if radix == 10 {
        match value {
            Obj::Int(_) | Obj::Real(_) => text_of(&value),
            _ => return fail("typecheck"),
        }
    } else {
        let whole = match value {
            Obj::Int(i) => i,
            Obj::Real(r) if (-2_147_483_648.0..=2_147_483_647.0).contains(&r.trunc()) => r as i32,
            Obj::Real(_) => return fail("rangecheck"),
            _ => return fail("typecheck"),
        };
        let radix = radix as u32;
        let mut rest = whole as u32;
        let mut digits = Vec::new();
        loop {
            let digit = char::from_digit(rest % radix, radix).unwrap_or('0');
            digits.push(digit.to_ascii_uppercase() as u8);
            rest /= radix;
            if rest == 0 {
                break;
            }
        }
        digits.reverse();
        digits
    };
    answer_text(m, target, &text)
}

fn close_array(m: &mut Machine) -> Res {
    let at = m.mark_at()?;
    let items = m.stack.split_off(at + 1);
    m.stack.pop();
    let array = m.new_array(items, false)?;
    m.answer(Obj::Array(array))
}

fn close_dict(m: &mut Machine) -> Res {
    let at = m.mark_at()?;
    if !(m.stack.len() - at - 1).is_multiple_of(2) {
        return fail("rangecheck");
    }
    let items = m.stack.split_off(at + 1);
    m.stack.pop();
    let dict = Dict::new(items.len() / 2);
    let mut items = items.into_iter();
    while let (Some(key), Some(value)) = (items.next(), items.next()) {
        dict.put(key, value).map_err(Fault::Error)?;
    }
    m.answer(Obj::Dict(dict))
}

fn astore(m: &mut Machine) -> Res {
    let array = m.pop_arr()?;
    if m.stack.len() < array.len {
        return fail("stackunderflow");
    }
    let items = m.stack.split_off(m.stack.len() - array.len);
    for (i, item) in items.into_iter().enumerate() {
        array.set(i, item);
    }
    m.answer(Obj::Array(array))
}

fn length(m: &mut Machine) -> Res {
    let n = match m.pop()? {
        Obj::Array(array) => array.len,
        Obj::Str(text) => text.len,
        Obj::Dict(dict) => dict.size(),
        Obj::Name(name, _) => name.len(),
        _ => return fail("typecheck"),
    };
    m.answer(int(n))
}

fn index_in(key: &Obj, len: usize) -> Res<usize> {
    match key {
        Obj::Int(i) => match usize::try_from(*i) {
            Ok(i) if i < len => Ok(i),
            _ => fail("rangecheck"),
        },
        _ => fail("typecheck"),
    }
}

fn get(m: &mut Machine) -> Res {
    let key = m.pop()?;
    let value = match m.pop()? {
        Obj::Array(array) => array.get(index_in(&key, array.len)?),
        Obj::Str(text) => Obj::Int(i32::from(text.get(index_in(&key, text.len)?))),
        Obj::Dict(dict) => dict.get(&key).ok_or(Fault::Error("undefined"))?,
        _ => return fail("typecheck"),
    };
    m.answer(value)
}

fn put(m: &mut Machine) -> Res {
    let value = m.pop()?;
    let key = m.pop()?;
    match m.pop()? {
        Obj::Array(array) => {
            array.set(index_in(&key, array.len)?, value);
            Ok(())
        }
        Obj::Str(text) => match value {
            Obj::Int(byte) => {
                text.set(index_in(&key, text.len)?, byte as u8);
                Ok(())
            }
            _ => fail("typecheck"),
        },
        Obj::Dict(dict) => dict.put(key, value).map_err(Fault::Error),
        _ => fail("typecheck"),
    }
}

/// A window `start`, `count` inside something `len` long.
fn window(start: i32, count: i32, len: usize) -> Res<(usize, usize)> {
    match (usize::try_from(start), usize::try_from(count)) {
        (Ok(start), Ok(count)) if start + count <= len => Ok((start, count)),
        _ => fail("rangecheck"),
    }
}

fn getinterval(m: &mut Machine) -> Res {
    let count = m.pop_int()?;
    let start = m.pop_int()?;
    match m.pop()? {
        Obj::Array(array) => {
            let (start, count) = window(start, count, array.len)?;
            m.answer(Obj::Array(array.interval(start, count)))
        }
        Obj::Str(text) => {
            let (start, count) = window(start, count, text.len)?;
            m.answer(Obj::Str(text.interval(start, count)))
        }
        _ => fail("typecheck"),
    }
}

fn putinterval(m: &mut Machine) -> Res {
    let source = m.pop()?;
    let start = m.pop_int()?;
    match (m.pop()?, source) {
        (Obj::Array(target), Obj::Array(source)) => {
            let (start, _) = window(start, source.len as i32, target.len)?;
            for (i, item) in source.to_vec().into_iter().enumerate() {
                target.set(start + i, item);
            }
            Ok(())
        }
        (Obj::Str(target), Obj::Str(source)) => {
            let (start, _) = window(start, source.len as i32, target.len)?;
            target.write(start, &source.to_vec());
            Ok(())
        }
        _ => fail("typecheck"),
    }
}

fn search(m: &mut Machine, anchored: bool) -> Res {
    let seek = m.pop_str()?.to_vec();
    let text = m.pop_str()?;
    let haystack = text.to_vec();
    let found = if anchored {
        haystack.starts_with(&seek).then_some(0)
    } else if seek.is_empty() {
        Some(0)
    } else {
        haystack
            .windows(seek.len())
            .position(|w| w == seek.as_slice())
    };
    let Some(at) = found else {
        m.push(Obj::Str(text));
        return m.answer(Obj::Bool(false));
    };
    let after = at + seek.len();
    m.push(Obj::Str(text.interval(after, text.len - after)));
    m.push(Obj::Str(text.interval(at, seek.len())));
    if !anchored {
        m.push(Obj::Str(text.interval(0, at)));
    }
    m.answer(Obj::Bool(true))
}

fn token(m: &mut Machine) -> Res {
    match m.pop()? {
        Obj::File(file, _) => match m.token(&file)? {
            Some(obj) => {
                m.push(obj);
                m.answer(Obj::Bool(true))
            }
            None => m.answer(Obj::Bool(false)),
        },
        Obj::Str(text) => {
            let source = Stream::bytes(text.to_vec().into(), &m.work);
            match m.token(&source)? {
                Some(obj) => {
                    let used = stream::position(&source).unwrap_or(text.len).min(text.len);
                    m.push(Obj::Str(text.interval(used, text.len - used)));
                    m.push(obj);
                    m.answer(Obj::Bool(true))
                }
                None => m.answer(Obj::Bool(false)),
            }
        }
        _ => fail("typecheck"),
    }
}

/// Replaces the names in a procedure that are operators with the
/// operators themselves, through nested procedures.
fn bind(m: &mut Machine) -> Res {
    let procedure = m.pop()?;
    if let Obj::Array(array) = &procedure {
        let mut seen = HashSet::new();
        bind_array(m, array, &mut seen, 0);
    }
    m.answer(procedure)
}

fn bind_array(m: &Machine, array: &Arr, seen: &mut HashSet<(usize, usize)>, depth: usize) {
    if depth > 64 || !seen.insert(array.id()) {
        return;
    }
    for i in 0..array.len {
        match array.get(i) {
            Obj::Name(name, true) => {
                if let Some(Obj::Op(op)) = m.lookup(&name) {
                    array.set(i, Obj::Op(op));
                }
            }
            Obj::Array(inner) if inner.exec => bind_array(m, &inner, seen, depth + 1),
            _ => {}
        }
    }
}

fn store(m: &mut Machine) -> Res {
    let value = m.pop()?;
    let key = m.pop()?;
    let dict = m.where_key(&key).unwrap_or_else(|| m.current_dict());
    dict.put(key, value).map_err(Fault::Error)
}

fn dictstack(m: &mut Machine) -> Res {
    let array = m.pop_arr()?;
    let count = m.dicts.len();
    if array.len < count {
        return fail("rangecheck");
    }
    for (i, dict) in m.dicts.iter().enumerate() {
        array.set(i, Obj::Dict(dict.clone()));
    }
    m.answer(Obj::Array(array.interval(0, count)))
}

fn execstack(m: &mut Machine) -> Res {
    let array = m.pop_arr()?;
    if array.len < m.depth {
        return fail("rangecheck");
    }
    for i in 0..m.depth {
        array.set(i, Obj::Null);
    }
    m.answer(Obj::Array(array.interval(0, m.depth)))
}

pub(super) fn clear_to_mark(m: &mut Machine) -> Res {
    let at = m.mark_at()?;
    m.stack.truncate(at);
    Ok(())
}

pub(super) static OPS: &[Entry] = &[
    (".error", |_| Err(Fault::Stop), 0),
    (".handleerror", |_| Ok(()), 0),
    // The operand stack.
    ("pop", |m| m.pop().map(drop), 1),
    (
        "exch",
        |m| {
            let b = m.pop()?;
            let a = m.pop()?;
            m.push(b);
            m.answer(a)
        },
        2,
    ),
    (
        "dup",
        |m| {
            let top = m.top()?.clone();
            m.answer(top)
        },
        1,
    ),
    ("copy", copy, 2),
    ("index", index, 1),
    ("roll", roll, 2),
    (
        "clear",
        |m| {
            m.stack.clear();
            Ok(())
        },
        0,
    ),
    ("count", |m| m.answer(int(m.stack.len())), 0),
    ("mark", |m| m.answer(Obj::Mark), 0),
    ("cleartomark", clear_to_mark, 0),
    (
        "counttomark",
        |m| {
            let at = m.mark_at()?;
            m.answer(int(m.stack.len() - at - 1))
        },
        0,
    ),
    // Arithmetic.
    ("add", |m| arith(m, i32::checked_add, |a, b| a + b), 2),
    ("sub", |m| arith(m, i32::checked_sub, |a, b| a - b), 2),
    ("mul", |m| arith(m, i32::checked_mul, |a, b| a * b), 2),
    (
        "div",
        |m| {
            let b = m.pop_num()?;
            let a = m.pop_num()?;
            if b == 0. {
                return fail("undefinedresult");
            }
            m.real(a / b)
        },
        2,
    ),
    ("idiv", |m| int_arith(m, i32::wrapping_div), 2),
    ("mod", |m| int_arith(m, i32::wrapping_rem), 2),
    ("neg", |m| unary(m, i32::checked_neg, |a| -a), 1),
    ("abs", |m| unary(m, i32::checked_abs, f64::abs), 1),
    ("ceiling", |m| rounding(m, f64::ceil), 1),
    ("floor", |m| rounding(m, f64::floor), 1),
    ("round", |m| rounding(m, |a| (a + 0.5).floor()), 1),
    ("truncate", |m| rounding(m, f64::trunc), 1),
    (
        "sqrt",
        |m| {
            let a = m.pop_num()?;
            if a < 0. {
                return fail("rangecheck");
            }
            m.real(a.sqrt())
        },
        1,
    ),
    (
        "atan",
        |m| {
            let den = m.pop_num()?;
            let num = m.pop_num()?;
            if num == 0. && den == 0. {
                return fail("undefinedresult");
            }
            m.real(num.atan2(den).to_degrees().rem_euclid(360.))
        },
        2,
    ),
    ("cos", |m| real_fn(m, |a| a.to_radians().cos()), 1),
    ("sin", |m| real_fn(m, |a| a.to_radians().sin()), 1),
    (
        "exp",
        |m| {
            let exponent = m.pop_num()?;
            let base = m.pop_num()?;
            m.real(base.powf(exponent))
        },
        2,
    ),
    ("ln", |m| positive_fn(m, f64::ln), 1),
    ("log", |m| positive_fn(m, f64::log10), 1),
    (
        "rand",
        |m| {
            m.seed = m.seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            m.answer(Obj::Int((m.seed >> 1) as i32))
        },
        0,
    ),
    (
        "srand",
        |m| {
            m.seed = m.pop_int()? as u32;
            Ok(())
        },
        1,
    ),
    ("rrand", |m| m.answer(Obj::Int(m.seed as i32)), 0),
    ("cvi", cvi, 1),
    (
        "cvr",
        |m| {
            let value = number_of(m.pop()?)?;
            m.real(value)
        },
        1,
    ),
    // Comparison and logic.
    (
        "eq",
        |m| {
            let b = m.pop()?;
            let a = m.pop()?;
            m.answer(Obj::Bool(same(&a, &b)))
        },
        2,
    ),
    (
        "ne",
        |m| {
            let b = m.pop()?;
            let a = m.pop()?;
            m.answer(Obj::Bool(!same(&a, &b)))
        },
        2,
    ),
    ("ge", |m| compare(m, Ordering::is_ge), 2),
    ("gt", |m| compare(m, Ordering::is_gt), 2),
    ("le", |m| compare(m, Ordering::is_le), 2),
    ("lt", |m| compare(m, Ordering::is_lt), 2),
    ("and", |m| logic(m, |a, b| a & b, |a, b| a & b), 2),
    ("or", |m| logic(m, |a, b| a | b, |a, b| a | b), 2),
    ("xor", |m| logic(m, |a, b| a ^ b, |a, b| a ^ b), 2),
    (
        "not",
        |m| match m.pop()? {
            Obj::Bool(b) => m.answer(Obj::Bool(!b)),
            Obj::Int(i) => m.answer(Obj::Int(!i)),
            _ => fail("typecheck"),
        },
        1,
    ),
    (
        "bitshift",
        |m| {
            let shift = m.pop_int()?;
            let value = m.pop_int()? as u32;
            let shifted = match shift {
                0..=31 => value << shift,
                -31..=-1 => value >> -shift,
                _ => 0,
            };
            m.answer(Obj::Int(shifted as i32))
        },
        2,
    ),
    // Control.
    (
        "exec",
        |m| {
            let obj = m.pop()?;
            m.exec(obj)
        },
        1,
    ),
    (
        "if",
        |m| {
            let procedure = m.pop_proc()?;
            if m.pop_bool()? {
                m.exec(procedure)
            } else {
                Ok(())
            }
        },
        2,
    ),
    (
        "ifelse",
        |m| {
            let no = m.pop_proc()?;
            let yes = m.pop_proc()?;
            let procedure = if m.pop_bool()? { yes } else { no };
            m.exec(procedure)
        },
        3,
    ),
    ("for", op_for, 4),
    (
        "repeat",
        |m| {
            let procedure = m.pop_proc()?;
            let times = m.pop_int()?;
            if times < 0 {
                return fail("rangecheck");
            }
            for _ in 0..times {
                if !iterate(m, &procedure)? {
                    break;
                }
            }
            Ok(())
        },
        2,
    ),
    (
        "loop",
        |m| {
            let procedure = m.pop_proc()?;
            while iterate(m, &procedure)? {}
            Ok(())
        },
        1,
    ),
    ("exit", |_| Err(Fault::Exit), 0),
    ("forall", forall, 2),
    ("stop", |_| Err(Fault::Stop), 0),
    ("stopped", stopped, 1),
    ("countexecstack", |m| m.answer(int(m.depth)), 0),
    ("execstack", execstack, 1),
    ("quit", |_| Err(Fault::Quit), 0),
    // Types and conversions.
    (
        "type",
        |m| {
            let obj = m.pop()?;
            m.answer(Obj::command(obj.type_name()))
        },
        1,
    ),
    (
        "cvlit",
        |m| {
            let obj = m.pop()?;
            m.answer(obj.with_exec(false))
        },
        1,
    ),
    (
        "cvx",
        |m| {
            let obj = m.pop()?;
            m.answer(obj.with_exec(true))
        },
        1,
    ),
    (
        "xcheck",
        |m| {
            let obj = m.pop()?;
            m.answer(Obj::Bool(obj.executable()))
        },
        1,
    ),
    ("executeonly", |m| m.top().map(drop), 1),
    ("noaccess", |m| m.top().map(drop), 1),
    ("readonly", |m| m.top().map(drop), 1),
    (
        "rcheck",
        |m| {
            m.pop()?;
            m.answer(Obj::Bool(true))
        },
        1,
    ),
    (
        "wcheck",
        |m| {
            let obj = m.pop()?;
            m.answer(Obj::Bool(!matches!(&obj, Obj::Dict(d) if d.readonly())))
        },
        1,
    ),
    (
        "cvn",
        |m| match m.pop()? {
            Obj::Str(text) => m.answer(Obj::Name(text.to_vec().into(), text.exec)),
            name @ Obj::Name(..) => m.answer(name),
            _ => fail("typecheck"),
        },
        1,
    ),
    ("cvs", cvs, 2),
    ("cvrs", cvrs, 3),
    // Arrays, strings and dictionaries.
    (
        "array",
        |m| {
            let n = m.pop_count(MAX_ARRAY)?;
            m.answer(Obj::Array(Arr::new(vec![Obj::Null; n], false)))
        },
        1,
    ),
    ("[", |m| m.answer(Obj::Mark), 0),
    ("]", close_array, 0),
    (
        "aload",
        |m| {
            let array = m.pop_arr()?;
            m.stack.extend(array.to_vec());
            m.answer(Obj::Array(array))
        },
        1,
    ),
    ("astore", astore, 1),
    ("length", length, 1),
    ("get", get, 2),
    ("put", put, 3),
    ("getinterval", getinterval, 3),
    ("putinterval", putinterval, 3),
    (
        "packedarray",
        |m| {
            let n = m.pop_count(MAX_ARRAY)?;
            if m.stack.len() < n {
                return fail("stackunderflow");
            }
            let items = m.stack.split_off(m.stack.len() - n);
            m.answer(Obj::Array(Arr::new(items, false)))
        },
        1,
    ),
    (
        "setpacking",
        |m| {
            m.packing = m.pop_bool()?;
            Ok(())
        },
        1,
    ),
    ("currentpacking", |m| m.answer(Obj::Bool(m.packing)), 0),
    (
        "string",
        |m| {
            let n = m.pop_count(MAX_STRING)?;
            m.answer(Obj::Str(Str::new(vec![0; n])))
        },
        1,
    ),
    ("anchorsearch", |m| search(m, true), 2),
    ("search", |m| search(m, false), 2),
    ("token", token, 1),
    (
        "dict",
        |m| {
            let n = m.pop_count(MAX_ARRAY)?;
            m.answer(Obj::Dict(Dict::new(n)))
        },
        1,
    ),
    ("<<", |m| m.answer(Obj::Mark), 0),
    (">>", close_dict, 0),
    (
        "begin",
        |m| {
            let dict = m.pop_dict()?;
            if m.dicts.len() >= MAX_DICTS {
                return fail("dictstackoverflow");
            }
            m.dicts.push(dict);
            Ok(())
        },
        1,
    ),
    (
        "end",
        |m| {
            if m.dicts.len() <= 3 {
                return fail("dictstackunderflow");
            }
            m.dicts.pop();
            Ok(())
        },
        0,
    ),
    (
        "def",
        |m| {
            let value = m.pop()?;
            let key = m.pop()?;
            m.def(key, value)
        },
        2,
    ),
    (
        "load",
        |m| {
            let key = m.pop()?;
            let value = m.lookup_key(&key).ok_or(Fault::Error("undefined"))?;
            m.answer(value)
        },
        1,
    ),
    ("store", store, 2),
    (
        "where",
        |m| {
            let key = m.pop()?;
            match m.where_key(&key) {
                Some(dict) => {
                    m.push(Obj::Dict(dict));
                    m.answer(Obj::Bool(true))
                }
                None => m.answer(Obj::Bool(false)),
            }
        },
        1,
    ),
    (
        "known",
        |m| {
            let key = m.pop()?;
            let dict = m.pop_dict()?;
            m.answer(Obj::Bool(dict.get(&key).is_some()))
        },
        2,
    ),
    (
        "undef",
        |m| {
            let key = m.pop()?;
            let dict = m.pop_dict()?;
            dict.remove(&key).map_err(Fault::Error)
        },
        2,
    ),
    (
        "currentdict",
        |m| {
            let dict = m.current_dict();
            m.answer(Obj::Dict(dict))
        },
        0,
    ),
    ("countdictstack", |m| m.answer(int(m.dicts.len())), 0),
    ("dictstack", dictstack, 1),
    (
        "cleardictstack",
        |m| {
            m.dicts.truncate(3);
            Ok(())
        },
        0,
    ),
    (
        "maxlength",
        |m| {
            let dict = m.pop_dict()?;
            m.answer(int(dict.capacity()))
        },
        1,
    ),
    ("bind", bind, 1),
];
