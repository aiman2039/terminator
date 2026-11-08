#![expect(
    clippy::arithmetic_side_effects,
    reason = "Fixture sizes are a few pixels."
)]

use super::*;

const RED: [u8; 4] = [0, 0, 255, 255];
const BLUE: [u8; 4] = [255, 0, 0, 255];

fn solid(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
    pixel.repeat(usize::try_from(width * height).unwrap())
}

fn pixel_at(canvas: &Canvas, x: u32, y: u32) -> [u8; 4] {
    let start = usize::try_from((y * canvas.width() + x) * 4).unwrap();
    let mut out = [0; 4];
    out.copy_from_slice(&canvas.pixels()[start..start + 4]);
    out
}

/// A layer at `(x, y)` of `(width, height)` with a packed stride.
fn layer(at: (i32, i32), size: (u32, u32), pixels: &[u8]) -> Layer<'_> {
    Layer {
        x: at.0,
        y: at.1,
        width: size.0,
        height: size.1,
        stride: usize::try_from(size.0 * 4).unwrap(),
        pixels,
        opaque: false,
    }
}

#[test]
fn child_surface_is_drawn_over_its_parent_at_its_offset() {
    let parent = solid(4, 4, RED);
    let child = solid(2, 2, BLUE);
    let mut canvas = Canvas::new(4, 4);
    canvas.draw(&layer((0, 0), (4, 4), &parent));
    canvas.draw(&layer((2, 1), (2, 2), &child));
    assert_eq!(pixel_at(&canvas, 0, 0), RED);
    assert_eq!(pixel_at(&canvas, 2, 1), BLUE);
    assert_eq!(pixel_at(&canvas, 3, 2), BLUE);
    assert_eq!(pixel_at(&canvas, 1, 2), RED);
}

#[test]
fn layer_outside_the_canvas_is_clipped_not_wrapped() {
    let child = solid(3, 3, BLUE);
    let mut canvas = Canvas::new(4, 4);
    canvas.draw(&layer((-2, 3), (3, 3), &child));
    assert_eq!(pixel_at(&canvas, 0, 3), BLUE);
    assert_eq!(pixel_at(&canvas, 1, 3), [0; 4]);
    assert_eq!(pixel_at(&canvas, 0, 2), [0; 4]);
    canvas.draw(&layer((10, 10), (3, 3), &child));
    canvas.draw(&layer((i32::MIN, i32::MAX), (3, 3), &child));
}

#[test]
fn half_transparent_pixel_blends_with_premultiplied_alpha() {
    let shadow = [0, 0, 0, 128];
    let mut canvas = Canvas::new(1, 1);
    canvas.draw(&layer((0, 0), (1, 1), &solid(1, 1, [200, 100, 50, 255])));
    canvas.draw(&layer((0, 0), (1, 1), &shadow));
    assert_eq!(pixel_at(&canvas, 0, 0), [99, 49, 24, 255]);
}

#[test]
fn xrgb_buffer_is_opaque_whatever_its_alpha_byte() {
    let junk = [10, 20, 30, 0];
    let mut canvas = Canvas::new(1, 1);
    canvas.draw(&Layer {
        opaque: true,
        ..layer((0, 0), (1, 1), &junk)
    });
    assert_eq!(pixel_at(&canvas, 0, 0), [10, 20, 30, 255]);
}

#[test]
fn short_buffer_is_skipped_without_panic() {
    let short = solid(2, 1, RED);
    let mut canvas = Canvas::new(2, 2);
    canvas.draw(&layer((0, 0), (2, 2), &short));
    assert_eq!(pixel_at(&canvas, 0, 0), RED);
    assert_eq!(pixel_at(&canvas, 0, 1), [0; 4]);
}

#[test]
fn padded_stride_rows_are_read_from_the_right_offset() {
    // 1x2 image, stride 8: one pixel then 4 bytes of padding per row.
    let pixels = [RED, [9; 4], BLUE, [9; 4]].concat();
    let mut canvas = Canvas::new(1, 2);
    canvas.draw(&Layer {
        stride: 8,
        ..layer((0, 0), (1, 2), &pixels)
    });
    assert_eq!(pixel_at(&canvas, 0, 0), RED);
    assert_eq!(pixel_at(&canvas, 0, 1), BLUE);
}

#[test]
fn huge_canvas_is_empty_instead_of_aborting() {
    let canvas = Canvas::new(u32::MAX, u32::MAX);
    assert!(canvas.pixels().is_empty());
    assert_eq!((canvas.width(), canvas.height()), (0, 0));
}

fn menu(anchor: u32, gravity: u32) -> Positioner {
    Positioner {
        width: 100,
        height: 50,
        anchor_x: 10,
        anchor_y: 20,
        anchor_width: 40,
        anchor_height: 10,
        anchor,
        gravity,
        offset_x: 0,
        offset_y: 0,
    }
}

#[test]
fn dropdown_menu_opens_below_the_left_of_its_button() {
    // anchor bottom_left, gravity bottom_right: the usual menu bar menu.
    assert_eq!(popup_position(&menu(6, 8)), (10, 30));
}

#[test]
fn popup_with_no_anchor_or_gravity_is_centered_on_the_anchor_rect() {
    assert_eq!(popup_position(&menu(0, 0)), (30 - 50, 25 - 25));
}

#[test]
fn popup_offset_is_added_last() {
    let mut p = menu(6, 8);
    p.offset_x = -3;
    p.offset_y = 4;
    assert_eq!(popup_position(&p), (7, 34));
}

#[test]
fn submenu_opens_to_the_right_of_its_item() {
    // anchor top_right, gravity bottom_right.
    assert_eq!(popup_position(&menu(7, 8)), (50, 20));
}
