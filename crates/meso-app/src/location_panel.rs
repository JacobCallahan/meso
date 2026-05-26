/*
 * Location manager panel.
 *
 * Shows a list of user-defined named locations with Activate / Delete
 * buttons, and a form to add new locations.
 *
 * Activating a location:
 *   1. Updates config.active_location, config.location_lat/lon
 *   2. Computes the nearest radar site via meso_data::geo::sites::nearest_site
 *   3. Calls `on_activate(site_id)` — supplied by app.rs to switch the radar
 *      site combo (which triggers the existing site-change chain)
 */

use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, Entry, Label, ListBox, ListBoxRow, Orientation, Separator, Window,
};
use std::cell::RefCell;
use std::rc::Rc;

use meso_data::geo::latlon::LatLon;
use meso_data::geo::sites;

use crate::config::{Config, NamedLocation};
use crate::panel::show_panel;

pub fn show_location_panel(
    parent: &impl IsA<Window>,
    shared_config: Rc<RefCell<Config>>,
    on_activate: impl Fn(&str) + 'static,
) {
    let on_activate: Rc<dyn Fn(&str)> = Rc::new(on_activate);

    let outer = GBox::new(Orientation::Vertical, 6);
    outer.set_margin_start(4);
    outer.set_margin_end(4);
    outer.set_margin_top(4);
    outer.set_margin_bottom(4);

    // ── Location list ─────────────────────────────────────────────────────────

    let list_label = Label::new(Some("Saved Locations"));
    list_label.add_css_class("heading");
    list_label.set_halign(gtk4::Align::Start);
    outer.append(&list_label);

    let listbox = ListBox::new();
    listbox.set_selection_mode(gtk4::SelectionMode::None);
    listbox.add_css_class("boxed-list");
    listbox.set_vexpand(true);
    outer.append(&listbox);

    // We need a Rc<dyn Fn()> that can call itself, so use a Rc<RefCell<Option<Rc<dyn Fn()>>>>
    #[allow(clippy::type_complexity)]
    let rebuild_fn: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    {
        let listbox = listbox.clone();
        let shared_config = Rc::clone(&shared_config);
        let on_activate = Rc::clone(&on_activate);
        let rebuild_fn_inner = Rc::clone(&rebuild_fn);

        let rebuild: Rc<dyn Fn()> = Rc::new(move || {
            // Remove all existing children
            while let Some(child) = listbox.first_child() {
                listbox.remove(&child);
            }

            let cfg = shared_config.borrow();
            let active_name = cfg.active_location.clone();

            for (idx, loc) in cfg.locations.iter().enumerate() {
                let is_active = loc.name == active_name;
                let row = build_location_row(
                    loc,
                    is_active,
                    idx,
                    Rc::clone(&shared_config),
                    Rc::clone(&on_activate),
                    Rc::clone(&rebuild_fn_inner),
                );
                listbox.append(&row);
            }
        });

        *rebuild_fn.borrow_mut() = Some(Rc::clone(&rebuild));
        // Initial population
        rebuild();
    }

    // ── Add location form ────────────────────────────────────────────────────

    outer.append(&Separator::new(Orientation::Horizontal));

    let add_label = Label::new(Some("Add Location"));
    add_label.add_css_class("heading");
    add_label.set_halign(gtk4::Align::Start);
    add_label.set_margin_top(4);
    outer.append(&add_label);

    let form = gtk4::Grid::new();
    form.set_row_spacing(4);
    form.set_column_spacing(8);

    let name_entry = Entry::new();
    name_entry.set_placeholder_text(Some("Location name"));
    name_entry.set_hexpand(true);

    let lat_entry = Entry::new();
    lat_entry.set_placeholder_text(Some("Latitude (e.g. 35.665)"));
    lat_entry.set_hexpand(true);

    let lon_entry = Entry::new();
    lon_entry.set_placeholder_text(Some("Longitude (e.g. -78.49)"));
    lon_entry.set_hexpand(true);

    form.attach(&Label::new(Some("Name")), 0, 0, 1, 1);
    form.attach(&name_entry, 1, 0, 1, 1);
    form.attach(&Label::new(Some("Lat")), 0, 1, 1, 1);
    form.attach(&lat_entry, 1, 1, 1, 1);
    form.attach(&Label::new(Some("Lon")), 0, 2, 1, 1);
    form.attach(&lon_entry, 1, 2, 1, 1);
    outer.append(&form);

    let error_label = Label::new(None);
    error_label.add_css_class("error");
    outer.append(&error_label);

    let add_btn = Button::with_label("+ Add Location");
    add_btn.set_halign(gtk4::Align::End);
    add_btn.set_margin_top(4);
    {
        let shared_config = Rc::clone(&shared_config);
        let rebuild_fn = Rc::clone(&rebuild_fn);
        let name_entry = name_entry.clone();
        let lat_entry = lat_entry.clone();
        let lon_entry = lon_entry.clone();
        let error_label = error_label.clone();
        add_btn.connect_clicked(move |_| {
            error_label.set_text("");
            let name = name_entry.text().trim().to_string();
            let lat_str = lat_entry.text();
            let lon_str = lon_entry.text();

            if name.is_empty() {
                error_label.set_text("Name is required.");
                return;
            }
            let lat = match lat_str.trim().parse::<f64>() {
                Ok(v) => v,
                Err(_) => {
                    error_label.set_text("Invalid latitude.");
                    return;
                }
            };
            let lon = match lon_str.trim().parse::<f64>() {
                Ok(v) => v,
                Err(_) => {
                    error_label.set_text("Invalid longitude.");
                    return;
                }
            };
            if !(-90.0..=90.0).contains(&lat) {
                error_label.set_text("Latitude out of range.");
                return;
            }
            if !(-180.0..=180.0).contains(&lon) {
                error_label.set_text("Longitude out of range.");
                return;
            }

            {
                let mut cfg = shared_config.borrow_mut();
                if cfg.locations.iter().any(|l| l.name == name) {
                    drop(cfg);
                    error_label.set_text("A location with that name already exists.");
                    return;
                }
                cfg.locations.push(NamedLocation { name, lat, lon });
            }

            name_entry.set_text("");
            lat_entry.set_text("");
            lon_entry.set_text("");

            if let Some(rb) = rebuild_fn.borrow().as_ref() {
                rb();
            }
        });
    }
    outer.append(&add_btn);

    let _ = rebuild_fn; // keep alive
    show_panel(parent, "Locations", 480, 560, outer);
}

#[allow(clippy::type_complexity)]
fn build_location_row(
    loc: &NamedLocation,
    is_active: bool,
    idx: usize,
    shared_config: Rc<RefCell<Config>>,
    on_activate: Rc<dyn Fn(&str)>,
    rebuild_fn: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
) -> ListBoxRow {
    let row = ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);

    let hbox = GBox::new(Orientation::Horizontal, 8);
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(6);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);

    // Location info label
    let info = Label::new(Some(&format!(
        "{} — {:.3}°{} {:.3}°{}",
        loc.name,
        loc.lat.abs(),
        if loc.lat >= 0.0 { "N" } else { "S" },
        loc.lon.abs(),
        if loc.lon >= 0.0 { "E" } else { "W" },
    )));
    info.set_hexpand(true);
    info.set_halign(gtk4::Align::Start);
    if is_active {
        info.add_css_class("accent");
    }
    hbox.append(&info);

    // Activate / Active button
    let activate_btn = if is_active {
        let b = Button::with_label("✓ Active");
        b.set_sensitive(false);
        b
    } else {
        let b = Button::with_label("● Activate");
        let name = loc.name.clone();
        let lat = loc.lat;
        let lon = loc.lon;
        let shared_config = Rc::clone(&shared_config);
        let on_activate = Rc::clone(&on_activate);
        let rebuild_fn = Rc::clone(&rebuild_fn);
        b.connect_clicked(move |_| {
            let site_id = {
                let mut cfg = shared_config.borrow_mut();
                cfg.active_location = name.clone();
                cfg.location_lat = lat;
                cfg.location_lon = lon;
                sites::nearest_site(&LatLon { lat, lon }, false).to_string()
            };
            on_activate(&site_id);
            if let Some(rb) = rebuild_fn.borrow().as_ref() {
                rb();
            }
        });
        b
    };
    hbox.append(&activate_btn);

    // Edit button
    let edit_btn = Button::with_label("✎ Edit");
    edit_btn.add_css_class("flat");
    {
        let row_c = row.clone();
        let loc_name = loc.name.clone();
        let loc_lat = loc.lat;
        let loc_lon = loc.lon;
        let shared_config_e = Rc::clone(&shared_config);
        let rebuild_fn_e = Rc::clone(&rebuild_fn);
        edit_btn.connect_clicked(move |_| {
            let edit_box = GBox::new(Orientation::Horizontal, 6);
            edit_box.set_margin_top(4);
            edit_box.set_margin_bottom(4);
            edit_box.set_margin_start(8);
            edit_box.set_margin_end(8);

            let name_e = Entry::new();
            name_e.set_text(&loc_name);
            name_e.set_placeholder_text(Some("Name"));
            name_e.set_width_chars(14);

            let lat_e = Entry::new();
            lat_e.set_text(&format!("{}", loc_lat));
            lat_e.set_placeholder_text(Some("Lat"));
            lat_e.set_width_chars(9);

            let lon_e = Entry::new();
            lon_e.set_text(&format!("{}", loc_lon));
            lon_e.set_placeholder_text(Some("Lon"));
            lon_e.set_width_chars(10);

            let save_btn = Button::with_label("Save");
            let cancel_btn = Button::with_label("Cancel");
            cancel_btn.add_css_class("flat");

            edit_box.append(&name_e);
            edit_box.append(&lat_e);
            edit_box.append(&lon_e);
            edit_box.append(&save_btn);
            edit_box.append(&cancel_btn);
            row_c.set_child(Some(&edit_box));

            // Cancel: just rebuild to restore original view
            {
                let rebuild_fn_c = Rc::clone(&rebuild_fn_e);
                cancel_btn.connect_clicked(move |_| {
                    if let Some(rb) = rebuild_fn_c.borrow().as_ref() {
                        rb();
                    }
                });
            }

            // Save: validate, update config, rebuild
            {
                let shared_config_s = Rc::clone(&shared_config_e);
                let rebuild_fn_s = Rc::clone(&rebuild_fn_e);
                let old_name = loc_name.clone();
                let name_es = name_e.clone();
                let lat_es = lat_e.clone();
                let lon_es = lon_e.clone();
                save_btn.connect_clicked(move |_| {
                    let new_name = name_es.text().trim().to_string();
                    let new_lat_s = lat_es.text();
                    let new_lon_s = lon_es.text();
                    if new_name.is_empty() {
                        return;
                    }
                    let new_lat = match new_lat_s.trim().parse::<f64>() {
                        Ok(v) => v,
                        Err(_) => return,
                    };
                    let new_lon = match new_lon_s.trim().parse::<f64>() {
                        Ok(v) => v,
                        Err(_) => return,
                    };
                    if !(-90.0..=90.0).contains(&new_lat) {
                        return;
                    }
                    if !(-180.0..=180.0).contains(&new_lon) {
                        return;
                    }

                    {
                        let mut cfg = shared_config_s.borrow_mut();
                        if let Some(loc) = cfg.locations.iter_mut().find(|l| l.name == old_name) {
                            loc.name = new_name.clone();
                            loc.lat = new_lat;
                            loc.lon = new_lon;
                        }
                        // If this was the active location, update active_location + lat/lon
                        if cfg.active_location == old_name {
                            cfg.active_location = new_name.clone();
                            cfg.location_lat = new_lat;
                            cfg.location_lon = new_lon;
                        }
                        let _ = cfg.save();
                    }

                    if let Some(rb) = rebuild_fn_s.borrow().as_ref() {
                        rb();
                    }
                });
            }
        });
    }
    hbox.append(&edit_btn);

    // Delete button (disabled for active location)
    let delete_btn = Button::with_label("✕");
    delete_btn.add_css_class("destructive-action");
    if is_active {
        delete_btn.set_sensitive(false);
        delete_btn.set_tooltip_text(Some("Cannot delete the active location"));
    } else {
        let shared_config = Rc::clone(&shared_config);
        let rebuild_fn = Rc::clone(&rebuild_fn);
        delete_btn.connect_clicked(move |_| {
            shared_config.borrow_mut().locations.remove(idx);
            if let Some(rb) = rebuild_fn.borrow().as_ref() {
                rb();
            }
        });
    }
    hbox.append(&delete_btn);

    row.set_child(Some(&hbox));
    row
}
