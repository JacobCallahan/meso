use gtk4::prelude::*;

/// Make a status label selectable and copyable via right-click.
pub fn enable_status_copy(status: &gtk4::Label) {
    status.set_selectable(true);
    status.set_wrap(true);
    status.set_tooltip_text(Some("Right-click to copy status text"));

    let status_c = status.clone();
    let copy_click = gtk4::GestureClick::new();
    copy_click.set_button(3);
    copy_click.connect_pressed(move |gesture, _n, _x, _y| {
        gesture.set_state(gtk4::EventSequenceState::Claimed);
        if let Some(display) = gtk4::gdk::Display::default() {
            display.clipboard().set_text(&status_c.text());
        }
    });
    status.add_controller(copy_click);
}
