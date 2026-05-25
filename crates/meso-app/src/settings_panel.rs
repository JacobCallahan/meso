/*
 * Settings panel.
 *
 * Exposes user-configurable settings grouped by category.
 * Changes are applied immediately to `shared_config`; they persist when the
 * application is closed normally (app.rs saves on close_request).
 *
 * Sections:
 *   Radar     — palette (ref/vel), animation frames
 *   Satellite — animation frames, default sector, default band
 *   Rendering — GPU toggle
 *   Cache     — per-type retention durations + "Clear All Cache" button
 */

use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, CheckButton, ComboBoxText, Label, Orientation, Separator, SpinButton,
    Window,
};
use std::cell::RefCell;
use std::rc::Rc;

use meso_data::goes::{BAND_CODES, SECTORS};
use meso_data::radar::color_palette::{REF_PALETTE_NAMES, VEL_PALETTE_NAMES};

use crate::config::Config;
use crate::panel::show_panel;
use crate::updraft_settings::show_updraft_settings;

pub fn show_settings_panel(parent: &impl IsA<Window>, shared_config: Rc<RefCell<Config>>) {
    let outer = GBox::new(Orientation::Vertical, 8);
    outer.set_margin_start(4);
    outer.set_margin_end(4);
    outer.set_margin_top(4);
    outer.set_margin_bottom(4);

    // ── Radar ─────────────────────────────────────────────────────────────────

    outer.append(&section_label("Radar"));

    let radar_grid = gtk4::Grid::new();
    radar_grid.set_row_spacing(6);
    radar_grid.set_column_spacing(12);
    radar_grid.set_margin_start(12);

    // Ref palette
    radar_grid.attach(&row_label("Reflectivity palette"), 0, 0, 1, 1);
    let ref_pal = ComboBoxText::new();
    for name in REF_PALETTE_NAMES {
        ref_pal.append(Some(name), name);
    }
    ref_pal.set_active_id(Some(&shared_config.borrow().radar_palette_ref));
    {
        let cfg = Rc::clone(&shared_config);
        ref_pal.connect_changed(move |combo| {
            if let Some(id) = combo.active_id() {
                cfg.borrow_mut().radar_palette_ref = id.to_string();
            }
        });
    }
    radar_grid.attach(&ref_pal, 1, 0, 1, 1);

    // Vel palette
    radar_grid.attach(&row_label("Velocity palette"), 0, 1, 1, 1);
    let vel_pal = ComboBoxText::new();
    for name in VEL_PALETTE_NAMES {
        vel_pal.append(Some(name), name);
    }
    vel_pal.set_active_id(Some(&shared_config.borrow().radar_palette_vel));
    {
        let cfg = Rc::clone(&shared_config);
        vel_pal.connect_changed(move |combo| {
            if let Some(id) = combo.active_id() {
                cfg.borrow_mut().radar_palette_vel = id.to_string();
            }
        });
    }
    radar_grid.attach(&vel_pal, 1, 1, 1, 1);

    // Animation frames
    radar_grid.attach(&row_label("Animation frames"), 0, 2, 1, 1);
    let radar_frames = SpinButton::with_range(2.0, 60.0, 1.0);
    radar_frames.set_value(shared_config.borrow().radar_anim_frames as f64);
    {
        let cfg = Rc::clone(&shared_config);
        radar_frames.connect_value_changed(move |spin| {
            cfg.borrow_mut().radar_anim_frames = spin.value() as u8;
        });
    }
    radar_grid.attach(&radar_frames, 1, 2, 1, 1);

    outer.append(&radar_grid);
    outer.append(&Separator::new(Orientation::Horizontal));

    // ── Satellite ─────────────────────────────────────────────────────────────

    outer.append(&section_label("Satellite"));

    let sat_grid = gtk4::Grid::new();
    sat_grid.set_row_spacing(6);
    sat_grid.set_column_spacing(12);
    sat_grid.set_margin_start(12);

    // Default sector
    sat_grid.attach(&row_label("Default sector"), 0, 0, 1, 1);
    let sector_combo = ComboBoxText::new();
    for s in SECTORS {
        sector_combo.append(Some(s.code), &format!("{} ({})", s.code, s.name));
    }
    sector_combo.set_active_id(Some(&shared_config.borrow().goes_sector));
    {
        let cfg = Rc::clone(&shared_config);
        sector_combo.connect_changed(move |combo| {
            if let Some(id) = combo.active_id() {
                cfg.borrow_mut().goes_sector = id.to_string();
            }
        });
    }
    sat_grid.attach(&sector_combo, 1, 0, 1, 1);

    // Default band
    sat_grid.attach(&row_label("Default band"), 0, 1, 1, 1);
    let band_combo = ComboBoxText::new();
    for code in BAND_CODES {
        band_combo.append(Some(code), code);
    }
    band_combo.set_active_id(Some(&shared_config.borrow().goes_band));
    {
        let cfg = Rc::clone(&shared_config);
        band_combo.connect_changed(move |combo| {
            if let Some(id) = combo.active_id() {
                cfg.borrow_mut().goes_band = id.to_string();
            }
        });
    }
    sat_grid.attach(&band_combo, 1, 1, 1, 1);

    // Animation frames
    sat_grid.attach(&row_label("Animation frames"), 0, 2, 1, 1);
    let sat_frames = SpinButton::with_range(2.0, 60.0, 1.0);
    sat_frames.set_value(shared_config.borrow().sat_anim_frames as f64);
    {
        let cfg = Rc::clone(&shared_config);
        sat_frames.connect_value_changed(move |spin| {
            cfg.borrow_mut().sat_anim_frames = spin.value() as u8;
        });
    }
    sat_grid.attach(&sat_frames, 1, 2, 1, 1);

    outer.append(&sat_grid);
    outer.append(&Separator::new(Orientation::Horizontal));

    // ── Rendering ─────────────────────────────────────────────────────────────

    outer.append(&section_label("Rendering"));

    let render_grid = gtk4::Grid::new();
    render_grid.set_row_spacing(6);
    render_grid.set_column_spacing(12);
    render_grid.set_margin_start(12);

    let gpu_check = CheckButton::with_label("Enable GPU rendering (requires restart)");
    gpu_check.set_active(shared_config.borrow().use_gpu);
    {
        let cfg = Rc::clone(&shared_config);
        gpu_check.connect_toggled(move |btn| {
            cfg.borrow_mut().use_gpu = btn.is_active();
        });
    }
    render_grid.attach(&gpu_check, 0, 0, 2, 1);
    outer.append(&render_grid);
    outer.append(&Separator::new(Orientation::Horizontal));

    // ── Cache Retention ───────────────────────────────────────────────────────

    outer.append(&section_label("Cache Retention"));

    let cache_grid = gtk4::Grid::new();
    cache_grid.set_row_spacing(6);
    cache_grid.set_column_spacing(12);
    cache_grid.set_margin_start(12);

    // Radar (hours)
    cache_grid.attach(&row_label("Radar (hours)"), 0, 0, 1, 1);
    let radar_cache = SpinButton::with_range(1.0, 168.0, 1.0);
    radar_cache.set_value(shared_config.borrow().cache_radar_hours as f64);
    {
        let cfg = Rc::clone(&shared_config);
        radar_cache.connect_value_changed(move |spin| {
            cfg.borrow_mut().cache_radar_hours = spin.value() as u32;
        });
    }
    cache_grid.attach(&radar_cache, 1, 0, 1, 1);

    // Satellite (hours)
    cache_grid.attach(&row_label("Satellite (hours)"), 0, 1, 1, 1);
    let sat_cache = SpinButton::with_range(1.0, 168.0, 1.0);
    sat_cache.set_value(shared_config.borrow().cache_sat_hours as f64);
    {
        let cfg = Rc::clone(&shared_config);
        sat_cache.connect_value_changed(move |spin| {
            cfg.borrow_mut().cache_sat_hours = spin.value() as u32;
        });
    }
    cache_grid.attach(&sat_cache, 1, 1, 1, 1);

    // Models (hours)
    cache_grid.attach(&row_label("Models (hours)"), 0, 2, 1, 1);
    let model_cache = SpinButton::with_range(1.0, 72.0, 1.0);
    model_cache.set_value(shared_config.borrow().cache_model_hours as f64);
    {
        let cfg = Rc::clone(&shared_config);
        model_cache.connect_value_changed(move |spin| {
            cfg.borrow_mut().cache_model_hours = spin.value() as u32;
        });
    }
    cache_grid.attach(&model_cache, 1, 2, 1, 1);

    // Observations (minutes)
    cache_grid.attach(&row_label("Observations (minutes)"), 0, 3, 1, 1);
    let obs_cache = SpinButton::with_range(10.0, 360.0, 5.0);
    obs_cache.set_value(shared_config.borrow().cache_obs_minutes as f64);
    {
        let cfg = Rc::clone(&shared_config);
        obs_cache.connect_value_changed(move |spin| {
            cfg.borrow_mut().cache_obs_minutes = spin.value() as u32;
        });
    }
    cache_grid.attach(&obs_cache, 1, 3, 1, 1);

    // Mesoanalysis (minutes)
    cache_grid.attach(&row_label("Mesoanalysis (minutes)"), 0, 4, 1, 1);
    let meso_cache = SpinButton::with_range(5.0, 120.0, 5.0);
    meso_cache.set_value(shared_config.borrow().cache_meso_minutes as f64);
    {
        let cfg = Rc::clone(&shared_config);
        meso_cache.connect_value_changed(move |spin| {
            cfg.borrow_mut().cache_meso_minutes = spin.value() as u32;
        });
    }
    cache_grid.attach(&meso_cache, 1, 4, 1, 1);

    outer.append(&cache_grid);

    // Clear cache button
    let clear_btn = Button::with_label("🗑 Clear All Cache");
    clear_btn.add_css_class("destructive-action");
    clear_btn.set_halign(gtk4::Align::Start);
    clear_btn.set_margin_start(12);
    clear_btn.set_margin_top(4);
    {
        let parent_win = parent.as_ref().clone();
        clear_btn.connect_clicked(move |btn| {
            let dialog = gtk4::AlertDialog::builder()
                .modal(true)
                .message("Clear all cached weather data?")
                .detail("This will remove all downloaded radar, satellite, model, and observation data from disk. Data will be re-fetched on next use.")
                .buttons(["Cancel", "Clear Cache"])
                .cancel_button(0)
                .default_button(0)
                .build();
            let btn_c = btn.clone();
            dialog.choose(Some(&parent_win), None::<&gtk4::gio::Cancellable>, move |result| {
                if result == Ok(1) {
                    std::thread::spawn(|| {
                        // Purge with 0 duration = remove everything
                        meso_data::cache::Cache::purge_old_global(std::time::Duration::ZERO);
                    });
                    btn_c.set_label("✓ Cache cleared");
                    btn_c.set_sensitive(false);
                }
            });
        });
    }
    outer.append(&clear_btn);

    // ── Updraft ───────────────────────────────────────────────────────────────
    outer.append(&Separator::new(Orientation::Horizontal));
    outer.append(&section_label("Updraft"));

    let updraft_grid = gtk4::Grid::new();
    updraft_grid.set_row_spacing(6);
    updraft_grid.set_column_spacing(12);
    updraft_grid.set_margin_start(12);

    // Enable toggle
    let updraft_check = CheckButton::with_label("Enable meso-updraft background caching");
    updraft_check.set_active(shared_config.borrow().updraft_enabled);
    {
        let cfg = Rc::clone(&shared_config);
        updraft_check.connect_toggled(move |btn| {
            cfg.borrow_mut().updraft_enabled = btn.is_active();
        });
    }
    updraft_grid.attach(&updraft_check, 0, 0, 2, 1);

    // Wake interval
    updraft_grid.attach(&row_label("Wake interval (seconds)"), 0, 1, 1, 1);
    let interval_spin = SpinButton::with_range(30.0, 3600.0, 30.0);
    interval_spin.set_value(shared_config.borrow().updraft_interval_secs as f64);
    {
        let cfg = Rc::clone(&shared_config);
        interval_spin.connect_value_changed(move |spin| {
            cfg.borrow_mut().updraft_interval_secs = spin.value() as u64;
        });
    }
    updraft_grid.attach(&interval_spin, 1, 1, 1, 1);

    outer.append(&updraft_grid);

    // Updraft settings button
    let updraft_btn = Button::with_label("Updraft Settings…");
    updraft_btn.set_halign(gtk4::Align::Start);
    updraft_btn.set_margin_start(12);
    updraft_btn.set_margin_top(4);
    {
        let parent_win = parent.as_ref().clone();
        updraft_btn.connect_clicked(move |_| {
            show_updraft_settings(&parent_win);
        });
    }
    outer.append(&updraft_btn);

    // Hint label
    let hint = Label::new(Some("Requires meso-updraft to be set up as a systemd user service."));
    hint.add_css_class("dim-label");
    hint.set_halign(gtk4::Align::Start);
    hint.set_margin_start(12);
    hint.set_margin_top(2);
    outer.append(&hint);

    show_panel(parent, "Settings", 520, 680, outer);
}

fn section_label(text: &str) -> Label {
    let l = Label::new(Some(text));
    l.add_css_class("title-4");
    l.set_halign(gtk4::Align::Start);
    l.set_margin_top(4);
    l
}

fn row_label(text: &str) -> Label {
    let l = Label::new(Some(text));
    l.set_halign(gtk4::Align::Start);
    l
}
