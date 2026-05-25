/*
 * Radar settings dialog: overlays + palette selection.
 */

use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, CheckButton, DropDown, Label, Orientation, ScrolledWindow, Separator,
    Window,
};
use std::cell::RefCell;
use std::rc::Rc;

use crate::config::Config;
use meso_data::radar::color_palette::{REF_PALETTE_NAMES, VEL_PALETTE_NAMES};

pub fn show_overlay_dialog(
    parent: &impl IsA<Window>,
    shared_cfg: Rc<RefCell<Config>>,
    on_apply: impl Fn() + 'static,
) {
    let win = gtk4::Window::new();
    win.set_title(Some("Radar Settings"));
    win.set_transient_for(Some(parent));
    win.set_modal(true);
    win.set_default_size(360, 400);

    let content = GBox::new(Orientation::Vertical, 8);
    content.set_margin_top(16);
    content.set_margin_bottom(8);
    content.set_margin_start(16);
    content.set_margin_end(16);

    let title = Label::new(Some("Radar Settings"));
    title.add_css_class("title-4");
    title.set_halign(gtk4::Align::Start);
    content.append(&title);

    let chk_warnings = CheckButton::with_label("Show Watches/Warnings");
    chk_warnings.set_active(shared_cfg.borrow().radar_show_warnings);
    content.append(&chk_warnings);

    let chk_tracks = CheckButton::with_label("Show Storm Tracks");
    chk_tracks.set_active(shared_cfg.borrow().radar_show_storm_tracks);
    content.append(&chk_tracks);

    let chk_major_roads = CheckButton::with_label("Show Major Roads");
    chk_major_roads.set_active(shared_cfg.borrow().radar_show_major_roads);
    content.append(&chk_major_roads);

    let chk_rings = CheckButton::with_label("Show Range Rings");
    chk_rings.set_active(shared_cfg.borrow().radar_show_rings);
    content.append(&chk_rings);

    let chk_track_points = CheckButton::with_label("Show Custom Track Points");
    chk_track_points.set_active(shared_cfg.borrow().radar_show_track_points);
    content.append(&chk_track_points);

    let chk_track_lines = CheckButton::with_label("Show Custom Track Lines");
    chk_track_lines.set_active(shared_cfg.borrow().radar_show_track_lines);
    content.append(&chk_track_lines);

    let chk_track_vector = CheckButton::with_label("Show Projected Track Vector");
    chk_track_vector.set_active(shared_cfg.borrow().radar_show_track_vector);
    content.append(&chk_track_vector);

    content.append(&Separator::new(Orientation::Horizontal));

    let palettes_hdr = Label::new(Some("Radar Palettes"));
    palettes_hdr.add_css_class("title-5");
    palettes_hdr.set_halign(gtk4::Align::Start);
    content.append(&palettes_hdr);

    let ref_row = GBox::new(Orientation::Horizontal, 8);
    let ref_lbl = Label::new(Some("Reflectivity"));
    ref_lbl.set_halign(gtk4::Align::Start);
    let ref_combo = DropDown::from_strings(REF_PALETTE_NAMES);
    if let Some(pos) = REF_PALETTE_NAMES
        .iter()
        .position(|&n| n == shared_cfg.borrow().radar_palette_ref)
    {
        ref_combo.set_selected(pos as u32);
    }
    ref_row.append(&ref_lbl);
    ref_row.append(&ref_combo);
    content.append(&ref_row);

    let vel_row = GBox::new(Orientation::Horizontal, 8);
    let vel_lbl = Label::new(Some("Velocity"));
    vel_lbl.set_halign(gtk4::Align::Start);
    let vel_combo = DropDown::from_strings(VEL_PALETTE_NAMES);
    if let Some(pos) = VEL_PALETTE_NAMES
        .iter()
        .position(|&n| n == shared_cfg.borrow().radar_palette_vel)
    {
        vel_combo.set_selected(pos as u32);
    }
    vel_row.append(&vel_lbl);
    vel_row.append(&vel_combo);
    content.append(&vel_row);

    content.append(&Separator::new(Orientation::Horizontal));

    let close_btn = Button::with_label("Close");
    close_btn.set_halign(gtk4::Align::End);
    close_btn.set_margin_top(4);
    close_btn.set_margin_bottom(8);
    content.append(&close_btn);

    let scroll = ScrolledWindow::builder()
        .child(&content)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();
    win.set_child(Some(&scroll));

    let cfg_close = Rc::clone(&shared_cfg);
    let win_c = win.clone();
    close_btn.connect_clicked(move |_| {
        {
            let mut cfg = cfg_close.borrow_mut();
            cfg.radar_show_warnings = chk_warnings.is_active();
            cfg.radar_show_storm_tracks = chk_tracks.is_active();
            cfg.radar_show_major_roads = chk_major_roads.is_active();
            cfg.radar_show_rings = chk_rings.is_active();
            cfg.radar_show_track_points = chk_track_points.is_active();
            cfg.radar_show_track_lines = chk_track_lines.is_active();
            cfg.radar_show_track_vector = chk_track_vector.is_active();
            if let Some(name) = REF_PALETTE_NAMES.get(ref_combo.selected() as usize) {
                cfg.radar_palette_ref = name.to_string();
            }
            if let Some(name) = VEL_PALETTE_NAMES.get(vel_combo.selected() as usize) {
                cfg.radar_palette_vel = name.to_string();
            }
        }
        on_apply();
        win_c.close();
    });

    win.present();
}
