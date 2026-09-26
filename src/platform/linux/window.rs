//! Window tiling for X11/EWMH. Public surface mirrors the macOS `window`
//! module: [`TilePosition`], [`rect_for`], [`tile_focused_window`].

use super::x11::{self, Rect};

#[derive(Debug, Clone, PartialEq)]
pub enum TilePosition {
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    LeftThird,
    CenterThird,
    RightThird,
    Maximize,
}

/// Compute the target rectangle for a tile position within work area `vf`.
/// X11 uses a top-left origin with y increasing downward, so "top" is at
/// the smaller y (unlike Cocoa).
pub fn rect_for(pos: &TilePosition, vf: Rect) -> Rect {
    let hw = vf.w / 2;
    let hh = vf.h / 2;
    let tw = vf.w / 3;
    match pos {
        TilePosition::LeftHalf => Rect {
            x: vf.x,
            y: vf.y,
            w: hw,
            h: vf.h,
        },
        TilePosition::RightHalf => Rect {
            x: vf.x + hw as i32,
            y: vf.y,
            w: vf.w - hw,
            h: vf.h,
        },
        TilePosition::TopHalf => Rect {
            x: vf.x,
            y: vf.y,
            w: vf.w,
            h: hh,
        },
        TilePosition::BottomHalf => Rect {
            x: vf.x,
            y: vf.y + hh as i32,
            w: vf.w,
            h: vf.h - hh,
        },
        TilePosition::TopLeft => Rect {
            x: vf.x,
            y: vf.y,
            w: hw,
            h: hh,
        },
        TilePosition::TopRight => Rect {
            x: vf.x + hw as i32,
            y: vf.y,
            w: vf.w - hw,
            h: hh,
        },
        TilePosition::BottomLeft => Rect {
            x: vf.x,
            y: vf.y + hh as i32,
            w: hw,
            h: vf.h - hh,
        },
        TilePosition::BottomRight => Rect {
            x: vf.x + hw as i32,
            y: vf.y + hh as i32,
            w: vf.w - hw,
            h: vf.h - hh,
        },
        TilePosition::LeftThird => Rect {
            x: vf.x,
            y: vf.y,
            w: tw,
            h: vf.h,
        },
        TilePosition::CenterThird => Rect {
            x: vf.x + tw as i32,
            y: vf.y,
            w: tw,
            h: vf.h,
        },
        TilePosition::RightThird => Rect {
            x: vf.x + (tw * 2) as i32,
            y: vf.y,
            w: vf.w - tw * 2,
            h: vf.h,
        },
        TilePosition::Maximize => vf,
    }
}

/// Tile the given toplevel window to `pos` on its current monitor.
/// Returns false on failure.
pub fn tile_focused_window(win: u32, pos: &TilePosition) -> bool {
    let Some(vf) = x11::monitor_workarea(win) else {
        return false;
    };
    let target = rect_for(pos, vf);
    x11::move_resize(win, target).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VF: Rect = Rect {
        x: 0,
        y: 23,
        w: 1920,
        h: 1057,
    };

    #[test]
    fn halves_cover_full_area() {
        let l = rect_for(&TilePosition::LeftHalf, VF);
        let r = rect_for(&TilePosition::RightHalf, VF);
        assert_eq!(l.w + r.w, VF.w);
        assert_eq!(l.x, VF.x);
        assert_eq!(l.x + l.w as i32, r.x);

        let t = rect_for(&TilePosition::TopHalf, VF);
        let b = rect_for(&TilePosition::BottomHalf, VF);
        assert_eq!(t.h + b.h, VF.h);
        assert_eq!(t.y + t.h as i32, b.y);
    }

    #[test]
    fn quarters_tile_without_overlap() {
        let tl = rect_for(&TilePosition::TopLeft, VF);
        let tr = rect_for(&TilePosition::TopRight, VF);
        let bl = rect_for(&TilePosition::BottomLeft, VF);
        let br = rect_for(&TilePosition::BottomRight, VF);
        assert_eq!(tl.w + tr.w, VF.w);
        assert_eq!(tl.h + bl.h, VF.h);
        assert_eq!(tl.x + tl.w as i32, tr.x);
        assert_eq!(bl.x + bl.w as i32, br.x);
        assert_eq!(tl.y + tl.h as i32, bl.y);
    }

    #[test]
    fn thirds_split_width_into_3() {
        let l = rect_for(&TilePosition::LeftThird, VF);
        let c = rect_for(&TilePosition::CenterThird, VF);
        let r = rect_for(&TilePosition::RightThird, VF);
        assert_eq!(l.w + c.w + r.w, VF.w);
        assert_eq!(l.x + l.w as i32, c.x);
        assert_eq!(c.x + c.w as i32, r.x);
    }

    #[test]
    fn maximize_equals_work_area() {
        assert_eq!(rect_for(&TilePosition::Maximize, VF), VF);
    }
}
