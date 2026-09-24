//! Illustrator's own operators, for Illustrator files of the PostScript
//! era that use them without the procedure set that defines them: paths
//! (`m l L c C v V y Y`), painting (`f F s S b B n N h H`) with separate
//! fill and stroke colours (`g G k K x X Xa XA Xx XX`), compound paths
//! (`*u *U`), groups (`u U q Q`), line attributes, and the rest read past
//! with a note. Their meanings are as the Adobe Illustrator File Format
//! Specification gives them, recalled rather than read from the document.
use super::graphics::{cmyk_to_rgb, grestore, pop_drop, rgb8, Shape, Stroke};
use super::{Fault, Machine, Res};

impl Machine {
    /// Illustrator's painting operators: the path is filled with the fill
    /// colour and stroked with the stroke colour; inside a compound path
    /// the subpaths gather until `*U` paints them as one.
    fn ai_paint(&mut self, close: bool, fill: bool, stroke: bool) -> Res {
        if close {
            self.close_path()?;
        }
        if let Some(paint) = &mut self.compound {
            *paint = (fill, stroke);
            return Ok(());
        }
        self.ai_draw(fill, stroke)
    }

    fn ai_draw(&mut self, fill: bool, stroke: bool) -> Res {
        let stroke = stroke.then(|| Stroke {
            colour: rgb8(self.gs.stroke_rgb),
            width: self.stroke_width(self.gs.ctm),
            join: self.gs.join,
            cap: self.gs.cap,
        });
        let fill = fill.then(|| (rgb8(self.gs.fill_rgb), self.evenodd));
        let path = self.take_path();
        if fill.is_some() || stroke.is_some() {
            self.emit(Shape { path, fill, stroke })?;
        }
        Ok(())
    }
}

/// Illustrator's operator `name`, for a file that uses it without defining
/// it (the procedure set left out), as the Adobe Illustrator File Format
/// Specification gives them; `None` when `name` is not one.
pub(super) fn operator(m: &mut Machine, name: &[u8]) -> Option<Res> {
    Some(match name {
        b"m" => m.pop_point().and_then(|p| m.move_to(p)),
        b"l" | b"L" => m.pop_point().and_then(|p| m.line_to(p)),
        b"c" | b"C" => (|| {
            let p = m.pop_point()?;
            let b = m.pop_point()?;
            let a = m.pop_point()?;
            m.curve_to(a, b, p)
        })(),
        // The first control point is the current point.
        b"v" | b"V" => (|| {
            let p = m.pop_point()?;
            let b = m.pop_point()?;
            let a = m.gs.point.ok_or(Fault::Error("nocurrentpoint"))?;
            m.curve_to(a, b, p)
        })(),
        // The second control point is the end point.
        b"y" | b"Y" => (|| {
            let p = m.pop_point()?;
            let a = m.pop_point()?;
            m.curve_to(a, p, p)
        })(),
        b"N" | b"H" => m.ai_paint(false, false, false),
        b"n" | b"h" => m.ai_paint(true, false, false),
        b"F" => m.ai_paint(false, true, false),
        b"f" => m.ai_paint(true, true, false),
        b"S" => m.ai_paint(false, false, true),
        b"s" => m.ai_paint(true, false, true),
        b"B" => m.ai_paint(false, true, true),
        b"b" => m.ai_paint(true, true, true),
        b"*u" => {
            m.compound = Some((false, false));
            Ok(())
        }
        b"*U" => match m.compound.take() {
            Some((fill, stroke)) => m.ai_draw(fill, stroke),
            None => Ok(()),
        },
        b"u" | b"U" | b"LB" => Ok(()),
        b"q" => m.push_state(None),
        b"Q" => grestore(m),
        b"W" => {
            m.art.clipped = true;
            Ok(())
        }
        b"g" | b"G" | b"k" | b"K" | b"x" | b"X" | b"Xa" | b"XA" | b"Xx" | b"XX" => {
            ai_colour(m, name)
        }
        b"w" | b"j" | b"J" | b"M" | b"d" | b"i" => {
            let operator = match name {
                b"w" => "setlinewidth",
                b"j" => "setlinejoin",
                b"J" => "setlinecap",
                b"M" => "setmiterlimit",
                b"d" => "setdash",
                _ => "setflat",
            };
            m.call_op(super::ops::find(operator))
        }
        b"XR" => m.pop_int().map(|rule| m.evenodd = rule == 1),
        b"A" | b"D" | b"O" | b"R" | b"Ap" | b"Ar" | b"Ln" | b"XW" => pop_drop(m, 1),
        b"Lb" => pop_drop(m, 10.min(m.stack.len())),
        b"To" | b"Tx" | b"Tj" | b"TX" => {
            m.art.text = true;
            pop_drop(m, 1.min(m.stack.len()))
        }
        b"Tp" => {
            m.art.text = true;
            pop_drop(m, 7.min(m.stack.len()))
        }
        b"TO" | b"TP" | b"T*" => {
            m.art.text = true;
            Ok(())
        }
        _ => return None,
    })
}

/// Illustrator's colours: `g`/`G` grey, `k`/`K` CMYK, `x`/`X` a named
/// CMYK colour and its tint (0 is full strength), `Xa`/`XA` RGB, `Xx`/`XX`
/// a named colour in CMYK (type 0) or RGB (type 1); lower case sets the
/// fill, upper case the stroke.
fn ai_colour(m: &mut Machine, name: &[u8]) -> Res {
    let rgb = match name {
        b"g" | b"G" => [m.pop_num()?.clamp(0., 1.); 3],
        b"k" | b"K" => {
            let v = m.pop_numbers(4)?;
            cmyk_to_rgb(v[0], v[1], v[2], v[3])
        }
        b"Xa" | b"XA" => {
            let v = m.pop_numbers(3)?;
            [v[0], v[1], v[2]]
        }
        b"x" | b"X" => {
            let tint = 1. - m.pop_num()?.clamp(0., 1.);
            m.pop()?;
            let v = m.pop_numbers(4)?;
            cmyk_to_rgb(v[0] * tint, v[1] * tint, v[2] * tint, v[3] * tint)
        }
        _ => {
            let kind = m.pop_int()?;
            let tint = 1. - m.pop_num()?.clamp(0., 1.);
            m.pop()?;
            if kind == 1 {
                let v = m.pop_numbers(3)?;
                [v[0], v[1], v[2]].map(|c| 1. - (1. - c) * tint)
            } else {
                let v = m.pop_numbers(4)?;
                cmyk_to_rgb(v[0] * tint, v[1] * tint, v[2] * tint, v[3] * tint)
            }
        }
    };
    if name[0].is_ascii_lowercase() {
        m.gs.fill_rgb = rgb;
    } else {
        m.gs.stroke_rgb = rgb;
    }
    Ok(())
}
