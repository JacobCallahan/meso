/*
 * Updraft Settings dialog.
 *
 * Shows all current radar and satellite subscriptions with "Unsubscribe"
 * buttons.  Changes are persisted immediately via save_subscriptions().
 */

use gtk4::prelude::*;
use gtk4::{Box as GBox, Button, Label, Orientation, Separator, Window};
use std::cell::RefCell;
use std::rc::Rc;

use meso_data::updraft::{load_subscriptions, save_subscriptions, Subscriptions};

use crate::panel::show_panel;

pub fn show_updraft_settings(parent: &impl IsA<Window>) {
    let subs = Rc::new(RefCell::new(load_subscriptions()));

    let outer = GBox::new(Orientation::Vertical, 8);
    outer.set_margin_start(4);
    outer.set_margin_end(4);
    outer.set_margin_top(4);
    outer.set_margin_bottom(4);

    // We keep a reference to `outer` so we can rebuild its contents.
    // Instead, we use a child container that gets rebuilt on each unsubscribe.
    let list_box = GBox::new(Orientation::Vertical, 4);
    outer.append(&list_box);

    build_list(&list_box, Rc::clone(&subs));

    show_panel(parent, "Updraft Subscriptions", 460, 400, outer);
}

fn build_list(list_box: &GBox, subs: Rc<RefCell<Subscriptions>>) {
    // Clear existing children
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }

    let borrowed = subs.borrow();

    // ── Radar subscriptions ───────────────────────────────────────────────────
    let radar_hdr = section_label("Radar");
    list_box.append(&radar_hdr);

    if borrowed.radar.is_empty() {
        list_box.append(&empty_label("No radar subscriptions."));
    } else {
        for (i, sub) in borrowed.radar.iter().enumerate() {
            let row = subscription_row(&format!("{} / {}", sub.station, sub.product), {
                let subs_c = Rc::clone(&subs);
                let list_box_c = list_box.clone();
                let idx = i;
                move || {
                    subs_c.borrow_mut().radar.remove(idx);
                    let _ = save_subscriptions(&subs_c.borrow());
                    build_list(&list_box_c, Rc::clone(&subs_c));
                }
            });
            list_box.append(&row);
        }
    }

    list_box.append(&Separator::new(Orientation::Horizontal));

    // ── Satellite subscriptions ───────────────────────────────────────────────
    let sat_hdr = section_label("Satellite");
    list_box.append(&sat_hdr);

    if borrowed.satellite.is_empty() {
        list_box.append(&empty_label("No satellite subscriptions."));
    } else {
        for (i, sub) in borrowed.satellite.iter().enumerate() {
            let row = subscription_row(&format!("{} / {}", sub.sector, sub.band), {
                let subs_c = Rc::clone(&subs);
                let list_box_c = list_box.clone();
                let idx = i;
                move || {
                    subs_c.borrow_mut().satellite.remove(idx);
                    let _ = save_subscriptions(&subs_c.borrow());
                    build_list(&list_box_c, Rc::clone(&subs_c));
                }
            });
            list_box.append(&row);
        }
    }
}

fn subscription_row(label_text: &str, on_remove: impl Fn() + 'static) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 8);
    row.set_margin_start(12);
    row.set_margin_top(2);
    row.set_margin_bottom(2);

    let lbl = Label::new(Some(label_text));
    lbl.set_hexpand(true);
    lbl.set_halign(gtk4::Align::Start);
    row.append(&lbl);

    let unsub_btn = Button::with_label("Unsubscribe");
    unsub_btn.connect_clicked(move |_| on_remove());
    row.append(&unsub_btn);

    row
}

fn section_label(text: &str) -> Label {
    let lbl = Label::new(Some(text));
    lbl.add_css_class("heading");
    lbl.set_halign(gtk4::Align::Start);
    lbl.set_margin_top(4);
    lbl.set_margin_start(4);
    lbl
}

fn empty_label(text: &str) -> Label {
    let lbl = Label::new(Some(text));
    lbl.add_css_class("dim-label");
    lbl.set_halign(gtk4::Align::Start);
    lbl.set_margin_start(16);
    lbl
}
