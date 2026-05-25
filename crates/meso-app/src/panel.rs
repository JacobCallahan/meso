/*
 * Reusable modal panel helper.
 *
 * `show_panel` opens a modal `gtk4::Window` anchored to a parent window.
 * The window contains the caller-supplied widget as its main content and a
 * "Close" button at the bottom.  The returned `gtk4::Window` can be stored by
 * the caller if it needs to close the panel programmatically.
 *
 * This pattern is reused by: location panel, settings panel, radar site picker,
 * obs site picker, etc.
 */

use gtk4::prelude::*;
use gtk4::{Box as GBox, Button, Label, Orientation, ScrolledWindow, Window};

/// Open a modal panel parented to `parent`.
///
/// `content` is placed in a `ScrolledWindow`; a "Close" button appears at the
/// bottom.  Returns the `gtk4::Window` so the caller can close it or hold a
/// reference.
pub fn show_panel(
    parent: &impl IsA<Window>,
    title: &str,
    width: i32,
    height: i32,
    content: impl IsA<gtk4::Widget>,
) -> Window {
    let win = Window::builder()
        .title(title)
        .modal(true)
        .transient_for(parent.as_ref())
        .default_width(width)
        .default_height(height)
        .resizable(true)
        .build();

    let outer = GBox::new(Orientation::Vertical, 0);

    // Title row
    let title_label = Label::new(Some(title));
    title_label.add_css_class("title-4");
    title_label.set_margin_top(12);
    title_label.set_margin_bottom(8);
    title_label.set_margin_start(12);
    title_label.set_margin_end(12);
    title_label.set_halign(gtk4::Align::Start);
    outer.append(&title_label);

    // Content in a scrolled window
    let sw = ScrolledWindow::new();
    sw.set_vexpand(true);
    sw.set_hexpand(true);
    sw.set_child(Some(&content));
    sw.set_margin_start(8);
    sw.set_margin_end(8);
    outer.append(&sw);

    // Close button row
    let btn_row = GBox::new(Orientation::Horizontal, 0);
    btn_row.set_halign(gtk4::Align::End);
    btn_row.set_margin_top(8);
    btn_row.set_margin_bottom(10);
    btn_row.set_margin_end(10);
    let close_btn = Button::with_label("Close");
    close_btn.add_css_class("suggested-action");
    let win_c = win.clone();
    close_btn.connect_clicked(move |_| win_c.close());
    btn_row.append(&close_btn);
    outer.append(&btn_row);

    win.set_child(Some(&outer));
    win.present();
    win
}
