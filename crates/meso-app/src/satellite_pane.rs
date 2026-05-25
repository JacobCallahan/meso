/*
 * GOES satellite viewer pane.
 *
 * Controls:
 *   Sector combo  → select GOES sector; auto-reload
 *   Band combo    → select ABI band; auto-reload
 *   ⟳ Refresh     → re-fetch current image
 *   Frames spin   → number of animation frames (2–60)
 *   ▶ Animate     → loop N frames at ~150 ms/frame
 *   + / −         → zoom buttons (bottom-right overlay)
 * Mouse scroll    → zoom centered on cursor
 * Click-drag      → pan
 */

use gdk_pixbuf::Pixbuf;
use glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, DrawingArea, DropDown, Label, Orientation, Overlay, Scale, SpinButton,
};

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use meso_data::goes::{self, BAND_CODES, BAND_LABELS, SECTORS};
use meso_data::updraft::{load_subscriptions, save_subscriptions};

use crate::config::Config;
use crate::runtime;
use crate::ui::enable_status_copy;

// ── State ─────────────────────────────────────────────────────────────────────

struct SatState {
    sector: String,
    band: String,
    current_pixbuf: Option<Pixbuf>,
    // Image-space zoom/pan (1.0 = fit to widget)
    zoom: f64,
    pan_x: f64,
    pan_y: f64,
    // Animation
    anim_frames: Vec<Pixbuf>,
    anim_timestamps: Vec<String>,
    anim_index: usize,
}

impl SatState {
    fn new(sector: &str, band: &str, zoom: f64, pan_x: f64, pan_y: f64) -> Self {
        SatState {
            sector: sector.to_string(),
            band: band.to_string(),
            current_pixbuf: None,
            zoom: zoom.max(0.1),
            pan_x,
            pan_y,
            anim_frames: Vec::new(),
            anim_timestamps: Vec::new(),
            anim_index: 0,
        }
    }
}

// ── Widget builder ────────────────────────────────────────────────────────────

pub fn build_satellite_pane(shared_cfg: Rc<RefCell<Config>>) -> GBox {
    let state = Rc::new(RefCell::new(SatState::new(
        &shared_cfg.borrow().goes_sector,
        &shared_cfg.borrow().goes_band,
        shared_cfg.borrow().sat_zoom,
        shared_cfg.borrow().sat_pan_x,
        shared_cfg.borrow().sat_pan_y,
    )));

    let anim_running = Rc::new(Cell::new(false));
    let anim_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let slider_updating: Rc<Cell<bool>> = Rc::new(Cell::new(false));

    let vbox = GBox::new(Orientation::Vertical, 0);

    // ── Toolbar ──────────────────────────────────────────────────────────────
    let toolbar = GBox::new(Orientation::Horizontal, 4);

    // Sector selector
    let sector_labels: Vec<&str> = SECTORS.iter().map(|s| s.name).collect();
    let sector_combo = DropDown::from_strings(&sector_labels);
    let current_sector = state.borrow().sector.clone();
    let sector_active = SECTORS.iter().position(|s| s.code == current_sector).unwrap_or(0);
    sector_combo.set_selected(sector_active as u32);
    sector_combo.set_tooltip_text(Some("Select GOES satellite sector"));
    toolbar.append(&sector_combo);

    // Band selector
    let band_combo = DropDown::from_strings(BAND_LABELS);
    let current_band = state.borrow().band.clone();
    let band_active = BAND_CODES.iter().position(|&c| c == current_band).unwrap_or(0);
    band_combo.set_selected(band_active as u32);
    band_combo.set_tooltip_text(Some("Select ABI spectral band"));
    toolbar.append(&band_combo);

    let refresh_btn = Button::with_label("⟳");
    refresh_btn.set_tooltip_text(Some("Reload current satellite image"));
    let anim_btn = Button::with_label("▶ Animate");
    anim_btn.set_tooltip_text(Some("Animate recent satellite frames"));

    // Subscribe button — ⚫ not subscribed, 🔵 subscribed
    let subscribe_btn = Button::with_label("⚫");
    subscribe_btn.set_tooltip_text(Some(
        "Subscribe to background caching for this sector/band (meso-updraft)",
    ));

    toolbar.append(&refresh_btn);
    toolbar.append(&subscribe_btn);

    // Frames spin button (after refresh/subscribe per UI spec)
    let frames_label = Label::new(Some("Frames:"));
    frames_label.set_tooltip_text(Some("Number of frames to fetch for animation"));
    toolbar.append(&frames_label);
    let frames_spin = SpinButton::with_range(2.0, 60.0, 1.0);
    frames_spin.set_value(shared_cfg.borrow().sat_anim_frames as f64);
    frames_spin.set_width_chars(3);
    toolbar.append(&frames_spin);

    toolbar.append(&anim_btn);
    vbox.append(&toolbar);

    // ── Drawing area ─────────────────────────────────────────────────────────
    let drawing_area = DrawingArea::new();
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);
    drawing_area.set_content_width(900);
    drawing_area.set_content_height(600);

    let state_draw = Rc::clone(&state);
    drawing_area.set_draw_func(move |_da, cr, w, h| {
        let st = state_draw.borrow();
        cr.set_source_rgb(0.05, 0.05, 0.05);
        let _ = cr.paint();

        if let Some(pb) = &st.current_pixbuf {
            let img_w = pb.width() as f64;
            let img_h = pb.height() as f64;
            // Fit image to widget at zoom 1.0
            let fit_scale = (w as f64 / img_w).min(h as f64 / img_h);
            let total_scale = fit_scale * st.zoom;
            // Center + pan offset
            let tx = w as f64 / 2.0 - (img_w / 2.0 + st.pan_x) * total_scale;
            let ty = h as f64 / 2.0 - (img_h / 2.0 + st.pan_y) * total_scale;
            cr.translate(tx, ty);
            cr.scale(total_scale, total_scale);
            cr.set_source_pixbuf(pb, 0.0, 0.0);
            let _ = cr.paint();
        } else {
            cr.set_source_rgb(0.4, 0.4, 0.4);
            cr.select_font_face(
                "Sans",
                gtk4::cairo::FontSlant::Normal,
                gtk4::cairo::FontWeight::Normal,
            );
            cr.set_font_size(16.0);
            let text = "Loading satellite...";
            let tx = (w as f64 - text.len() as f64 * 8.0) / 2.0;
            cr.move_to(tx, h as f64 / 2.0);
            let _ = cr.show_text(text);
        }
    });

    // Track mouse for cursor-centered zoom
    let mouse_pos = Rc::new(Cell::new((0.0f64, 0.0f64)));
    let motion_ctrl = gtk4::EventControllerMotion::new();
    {
        let mp = Rc::clone(&mouse_pos);
        motion_ctrl.connect_motion(move |_, x, y| mp.set((x, y)));
    }
    drawing_area.add_controller(motion_ctrl);

    // Scroll → zoom centered on cursor
    let scroll_ctrl = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    {
        let state_s = Rc::clone(&state);
        let da_s = drawing_area.clone();
        let mp_s = Rc::clone(&mouse_pos);
        let cfg_s = Rc::clone(&shared_cfg);
        scroll_ctrl.connect_scroll(move |_, _dx, dy| {
            let factor = if dy < 0.0 { 1.15 } else { 1.0 / 1.15 };
            {
                let mut st = state_s.borrow_mut();
                zoom_image_around(
                    &mut st,
                    mp_s.get(),
                    factor,
                    da_s.width() as f64,
                    da_s.height() as f64,
                );
                let mut cfg = cfg_s.borrow_mut();
                cfg.sat_zoom = st.zoom;
                cfg.sat_pan_x = st.pan_x;
                cfg.sat_pan_y = st.pan_y;
            }
            da_s.queue_draw();
            glib::Propagation::Stop
        });
    }
    drawing_area.add_controller(scroll_ctrl);

    // Drag → pan
    let drag_ctrl = gtk4::GestureDrag::new();
    let last_offset = Rc::new(RefCell::new((0.0f64, 0.0f64)));
    {
        let state_d = Rc::clone(&state);
        let da_d = drawing_area.clone();
        let last_off = Rc::clone(&last_offset);
        let cfg_d = Rc::clone(&shared_cfg);
        drag_ctrl.connect_drag_update(move |_g, dx, dy| {
            let (prev_dx, prev_dy) = *last_off.borrow();
            let ddx = dx - prev_dx;
            let ddy = dy - prev_dy;
            *last_off.borrow_mut() = (dx, dy);
            {
                let mut st = state_d.borrow_mut();
                // Convert pixel delta to image-space delta
                if let Some(pb) = &st.current_pixbuf {
                    let img_w = pb.width() as f64;
                    let img_h = pb.height() as f64;
                    let fit_scale = (da_d.width() as f64 / img_w).min(da_d.height() as f64 / img_h);
                    let total_scale = fit_scale * st.zoom;
                    st.pan_x -= ddx / total_scale;
                    st.pan_y -= ddy / total_scale;
                }
                let mut cfg = cfg_d.borrow_mut();
                cfg.sat_zoom = st.zoom;
                cfg.sat_pan_x = st.pan_x;
                cfg.sat_pan_y = st.pan_y;
            }
            da_d.queue_draw();
        });
        let last_off2 = Rc::clone(&last_offset);
        drag_ctrl.connect_drag_end(move |_, _, _| {
            *last_off2.borrow_mut() = (0.0, 0.0);
        });
    }
    drawing_area.add_controller(drag_ctrl);

    // ── Overlay: zoom ± buttons ───────────────────────────────────────────────
    let overlay = Overlay::new();
    overlay.set_child(Some(&drawing_area));

    let zoom_box = GBox::new(Orientation::Horizontal, 2);
    zoom_box.set_halign(gtk4::Align::End);
    zoom_box.set_valign(gtk4::Align::End);
    zoom_box.set_margin_end(8);
    zoom_box.set_margin_bottom(8);
    zoom_box.add_css_class("linked");

    let zoom_in_btn = Button::with_label("+");
    let zoom_out_btn = Button::with_label("−");
    zoom_box.append(&zoom_out_btn);
    zoom_box.append(&zoom_in_btn);
    overlay.add_overlay(&zoom_box);

    {
        let state_zi = Rc::clone(&state);
        let da_zi = drawing_area.clone();
        let cfg_zi = Rc::clone(&shared_cfg);
        zoom_in_btn.connect_clicked(move |_| {
            {
                let mut st = state_zi.borrow_mut();
                let cx = da_zi.width() as f64 / 2.0;
                let cy = da_zi.height() as f64 / 2.0;
                zoom_image_around(
                    &mut st,
                    (cx, cy),
                    1.5,
                    da_zi.width() as f64,
                    da_zi.height() as f64,
                );
                let mut cfg = cfg_zi.borrow_mut();
                cfg.sat_zoom = st.zoom;
                cfg.sat_pan_x = st.pan_x;
                cfg.sat_pan_y = st.pan_y;
            }
            da_zi.queue_draw();
        });
    }
    {
        let state_zo = Rc::clone(&state);
        let da_zo = drawing_area.clone();
        let cfg_zo = Rc::clone(&shared_cfg);
        zoom_out_btn.connect_clicked(move |_| {
            {
                let mut st = state_zo.borrow_mut();
                let cx = da_zo.width() as f64 / 2.0;
                let cy = da_zo.height() as f64 / 2.0;
                zoom_image_around(
                    &mut st,
                    (cx, cy),
                    1.0 / 1.5,
                    da_zo.width() as f64,
                    da_zo.height() as f64,
                );
                let mut cfg = cfg_zo.borrow_mut();
                cfg.sat_zoom = st.zoom;
                cfg.sat_pan_x = st.pan_x;
                cfg.sat_pan_y = st.pan_y;
            }
            da_zo.queue_draw();
        });
    }

    vbox.append(&overlay);

    // ── Timeline scrubber ────────────────────────────────────────────────────
    let timeline = Scale::with_range(Orientation::Horizontal, 0.0, 1.0, 1.0);
    timeline.set_hexpand(true);
    timeline.set_draw_value(false);
    timeline.set_sensitive(false);
    timeline.set_margin_start(4);
    timeline.set_margin_end(4);
    vbox.append(&timeline);

    // Timeline scrubber value_changed handler
    {
        let state_tl = Rc::clone(&state);
        let da_tl = drawing_area.clone();
        let su_tl = Rc::clone(&slider_updating);
        timeline.connect_value_changed(move |scale| {
            if su_tl.get() {
                return;
            }
            let idx = scale.value() as usize;
            let mut st = state_tl.borrow_mut();
            let n = st.anim_frames.len();
            if n == 0 || idx >= n {
                return;
            }
            st.anim_index = idx;
            st.current_pixbuf = Some(st.anim_frames[idx].clone());
            drop(st);
            da_tl.queue_draw();
        });
    }

    // ── Status ───────────────────────────────────────────────────────────────
    let status = Label::new(Some("Select sector and band"));
    status.set_halign(gtk4::Align::Start);
    enable_status_copy(&status);
    vbox.append(&status);

    // Load on startup
    load_sat_image(
        Rc::clone(&state),
        drawing_area.clone(),
        status.clone(),
        vec![refresh_btn.clone(), anim_btn.clone()],
    );

    // Auto-refresh latest satellite image every 5 minutes when not animating
    schedule_sat_auto_refresh(
        Rc::clone(&state),
        drawing_area.clone(),
        status.clone(),
        vec![refresh_btn.clone(), anim_btn.clone()],
        Rc::clone(&anim_running),
    );

    // ── Wire controls ─────────────────────────────────────────────────────────

    // Sector combo
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let band_c = band_combo.clone();
        let cfg_c = Rc::clone(&shared_cfg);
        let ar_c = Rc::clone(&anim_running);
        let at_c = Rc::clone(&anim_timer);
        let anim_btn_sec = anim_btn.clone();
        let refresh_btn_sec = refresh_btn.clone();
        let tl_sec = timeline.clone();
        sector_combo.connect_selected_notify(move |combo| {
            let idx = combo.selected() as usize;
            if let Some(sec) = SECTORS.get(idx) {
                let code = sec.code.to_string();
                if ar_c.get() {
                    ar_c.set(false);
                    if let Some(id) = at_c.borrow_mut().take() {
                        id.remove();
                    }
                    anim_btn_sec.set_label("▶ Animate");
                }
                // Clear animation state and reset timeline
                {
                    let mut st = state_c.borrow_mut();
                    st.anim_frames.clear();
                    st.anim_timestamps.clear();
                    st.anim_index = 0;
                }
                tl_sec.set_sensitive(false);
                cfg_c.borrow_mut().goes_sector = code.clone();
                {
                    let mut st = state_c.borrow_mut();
                    st.sector = code;
                    let band_idx = band_c.selected() as usize;
                    if let Some(&bc) = BAND_CODES.get(band_idx) {
                        st.band = bc.to_string();
                    }
                }
                let btns = vec![refresh_btn_sec.clone(), anim_btn_sec.clone()];
                load_sat_image(Rc::clone(&state_c), da_c.clone(), st_c.clone(), btns);
            }
        });
    }

    // Band combo
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let sec_c = sector_combo.clone();
        let cfg_c = Rc::clone(&shared_cfg);
        let ar_c = Rc::clone(&anim_running);
        let at_c = Rc::clone(&anim_timer);
        let anim_btn_band = anim_btn.clone();
        let refresh_btn_band = refresh_btn.clone();
        let tl_band = timeline.clone();
        band_combo.connect_selected_notify(move |combo| {
            let idx = combo.selected() as usize;
            if let Some(&code) = BAND_CODES.get(idx) {
                let code = code.to_string();
                if ar_c.get() {
                    ar_c.set(false);
                    if let Some(id) = at_c.borrow_mut().take() {
                        id.remove();
                    }
                    anim_btn_band.set_label("▶ Animate");
                }
                // Clear animation state and reset timeline
                {
                    let mut st = state_c.borrow_mut();
                    st.anim_frames.clear();
                    st.anim_timestamps.clear();
                    st.anim_index = 0;
                }
                tl_band.set_sensitive(false);
                cfg_c.borrow_mut().goes_band = code.clone();
                {
                    let mut st = state_c.borrow_mut();
                    st.band = code;
                    let sec_idx = sec_c.selected() as usize;
                    if let Some(s) = SECTORS.get(sec_idx) {
                        st.sector = s.code.to_string();
                    }
                }
                let btns = vec![refresh_btn_band.clone(), anim_btn_band.clone()];
                load_sat_image(Rc::clone(&state_c), da_c.clone(), st_c.clone(), btns);
            }
        });
    }

    // Refresh button
    {
        let state_r = Rc::clone(&state);
        let da_r = drawing_area.clone();
        let stat_r = status.clone();
        let btns_r = vec![refresh_btn.clone(), anim_btn.clone()];
        refresh_btn.connect_clicked(move |_| {
            load_sat_image(
                Rc::clone(&state_r),
                da_r.clone(),
                stat_r.clone(),
                btns_r.clone(),
            );
        });
    }

    // Frames spin
    {
        let cfg_c = Rc::clone(&shared_cfg);
        frames_spin.connect_value_changed(move |spin| {
            cfg_c.borrow_mut().sat_anim_frames = spin.value() as u8;
        });
    }

    // Animation toggle — three states:
    //   1) Running → pause (stop timer, keep frames)
    //   2) Paused with frames → resume (restart timer, no re-fetch)
    //   3) No frames → fetch + animate
    {
        let state_a = Rc::clone(&state);
        let da_a = drawing_area.clone();
        let st_a = status.clone();
        let ar_a = Rc::clone(&anim_running);
        let at_a = Rc::clone(&anim_timer);
        let anim_btn_c = anim_btn.clone();
        let frames_s = frames_spin.clone();
        let timeline_a = timeline.clone();
        let su_a = Rc::clone(&slider_updating);
        anim_btn.connect_clicked(move |_| {
            if ar_a.get() {
                // Pause
                ar_a.set(false);
                if let Some(id) = at_a.borrow_mut().take() {
                    id.remove();
                }
                anim_btn_c.set_label("▶ Animate");
            } else if !state_a.borrow().anim_frames.is_empty() {
                // Resume
                ar_a.set(true);
                anim_btn_c.set_label("⏸ Pause");
                start_sat_timer(
                    Rc::clone(&state_a),
                    da_a.clone(),
                    Rc::clone(&ar_a),
                    Rc::clone(&at_a),
                    timeline_a.clone(),
                    Rc::clone(&su_a),
                );
            } else {
                // Fetch new frames
                let sector = state_a.borrow().sector.clone();
                let band = state_a.borrow().band.clone();
                let frame_count = frames_s.value() as usize;
                ar_a.set(true);
                anim_btn_c.set_label("⏸ Pause");
                fetch_sat_animation(
                    Rc::clone(&state_a),
                    da_a.clone(),
                    st_a.clone(),
                    Rc::clone(&ar_a),
                    Rc::clone(&at_a),
                    sector,
                    band,
                    frame_count,
                    timeline_a.clone(),
                    Rc::clone(&su_a),
                    anim_btn_c.clone(),
                );
            }
        });
    }

    // Subscribe button wiring
    {
        let sector_c = sector_combo.clone();
        let band_c = band_combo.clone();
        let btn = subscribe_btn.clone();

        // Initial state
        {
            let sector = SECTORS.get(sector_c.selected() as usize).map(|s| s.code.to_string()).unwrap_or_default();
            let band = BAND_CODES.get(band_c.selected() as usize).map(|&s| s.to_string()).unwrap_or_default();
            if !sector.is_empty() && !band.is_empty() {
                let subs = load_subscriptions();
                btn.set_label(if subs.is_sat_subscribed(&sector, &band) { "🔵" } else { "⚫" });
            }
        }

        // Re-check when sector changes
        {
            let band_c2 = band_combo.clone();
            let btn2 = subscribe_btn.clone();
            sector_combo.connect_selected_notify(move |combo| {
                let sector = SECTORS.get(combo.selected() as usize).map(|s| s.code.to_string()).unwrap_or_default();
                let band = BAND_CODES.get(band_c2.selected() as usize).map(|&s| s.to_string()).unwrap_or_default();
                if !sector.is_empty() && !band.is_empty() {
                    let subs = load_subscriptions();
                    btn2.set_label(if subs.is_sat_subscribed(&sector, &band) { "🔵" } else { "⚫" });
                }
            });
        }

        // Re-check when band changes
        {
            let sector_c3 = sector_combo.clone();
            let btn3 = subscribe_btn.clone();
            band_combo.connect_selected_notify(move |combo| {
                let sector = SECTORS.get(sector_c3.selected() as usize).map(|s| s.code.to_string()).unwrap_or_default();
                let band = BAND_CODES.get(combo.selected() as usize).map(|&s| s.to_string()).unwrap_or_default();
                if !sector.is_empty() && !band.is_empty() {
                    let subs = load_subscriptions();
                    btn3.set_label(if subs.is_sat_subscribed(&sector, &band) { "🔵" } else { "⚫" });
                }
            });
        }

        // Toggle on click
        subscribe_btn.connect_clicked(move |_| {
            let sector = SECTORS.get(sector_c.selected() as usize).map(|s| s.code.to_string()).unwrap_or_default();
            let band = BAND_CODES.get(band_c.selected() as usize).map(|&s| s.to_string()).unwrap_or_default();
            if sector.is_empty() || band.is_empty() {
                return;
            }
            let mut subs = load_subscriptions();
            let now_subscribed = subs.toggle_sat(&sector, &band);
            let _ = save_subscriptions(&subs);
            btn.set_label(if now_subscribed { "🔵" } else { "⚫" });
        });
    }

    vbox
}

// ── Auto-refresh (every 5 minutes, only when not animating) ──────────────────

fn schedule_sat_auto_refresh(
    state: Rc<RefCell<SatState>>,
    da: DrawingArea,
    status: Label,
    btns: Vec<Button>,
    anim_running: Rc<Cell<bool>>,
) {
    glib::timeout_add_local(std::time::Duration::from_secs(300), move || {
        if !anim_running.get() {
            load_sat_image(Rc::clone(&state), da.clone(), status.clone(), btns.clone());
        }
        glib::ControlFlow::Continue
    });
}

// ── Image-space zoom helper ───────────────────────────────────────────────────

/// Zoom the satellite image around a screen point (wx, wy) in widget pixels.
fn zoom_image_around(
    st: &mut SatState,
    (wx, wy): (f64, f64),
    factor: f64,
    widget_w: f64,
    widget_h: f64,
) {
    if let Some(pb) = &st.current_pixbuf {
        let img_w = pb.width() as f64;
        let img_h = pb.height() as f64;
        let fit_scale = (widget_w / img_w).min(widget_h / img_h);
        let total_scale = fit_scale * st.zoom;

        // Image-space coords of the cursor before zoom
        let img_x = (wx - widget_w / 2.0) / total_scale + img_w / 2.0 + st.pan_x;
        let img_y = (wy - widget_h / 2.0) / total_scale + img_h / 2.0 + st.pan_y;

        st.zoom = (st.zoom * factor).clamp(0.1, 20.0);

        // Recompute at new zoom: keep img_x/img_y at the same screen point
        let new_total_scale = fit_scale * st.zoom;
        st.pan_x = img_x - img_w / 2.0 - (wx - widget_w / 2.0) / new_total_scale;
        st.pan_y = img_y - img_h / 2.0 - (wy - widget_h / 2.0) / new_total_scale;
    } else {
        st.zoom = (st.zoom * factor).clamp(0.1, 20.0);
    }
}

// ── Data loading ──────────────────────────────────────────────────────────────

fn load_sat_image(state: Rc<RefCell<SatState>>, da: DrawingArea, status: Label, btns: Vec<Button>) {
    let sector = state.borrow().sector.clone();
    let band = state.borrow().band.clone();
    let url = goes::image_url(&sector, &band);
    for b in &btns {
        b.set_sensitive(false);
    }
    status.set_text(&format!("Fetching {sector}/{band}..."));

    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            goes::fetch_image(&client, &url).await
        },
        move |result| {
            for b in &btns {
                b.set_sensitive(true);
            }
            match result {
                Ok(bytes) => match bytes_to_pixbuf(&bytes) {
                    Some(pb) => {
                        let mut st = state.borrow_mut();
                        st.current_pixbuf = Some(pb);
                        st.anim_frames.clear();
                        st.zoom = 1.0;
                        st.pan_x = 0.0;
                        st.pan_y = 0.0;
                        drop(st);
                        da.queue_draw();
                        status.set_text("Ready");
                    }
                    None => status.set_text("Failed to decode image"),
                },
                Err(e) => status.set_text(&format!("Error: {e}")),
            }
        },
    );
}

fn bytes_to_pixbuf(bytes: &[u8]) -> Option<Pixbuf> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    loader.pixbuf()
}

/// Extract a display timestamp from a GOES CDN URL.
/// GOES filenames contain UTC timestamps like "20241205T120000Z" or "20241205_1200".
fn sat_timestamp_from_url(url: &str) -> String {
    // Try format: YYYYMMDDTHHMMSSZ (e.g. "20241205T120000Z")
    if let Some(pos) = url.rfind('/') {
        let filename = &url[pos + 1..];
        // Look for 8-digit date followed by T and time
        let chars: Vec<char> = filename.chars().collect();
        for i in 0..chars.len().saturating_sub(14) {
            if chars[i..i + 8].iter().all(|c| c.is_ascii_digit())
                && chars.get(i + 8) == Some(&'T')
                && chars[i + 9..i + 15].iter().all(|c| c.is_ascii_digit())
            {
                let date = &filename[i..i + 8];
                let time = &filename[i + 9..i + 15];
                // Format as "YYYY-MM-DD HH:MM UTC"
                return format!(
                    "{}-{}-{} {}:{} UTC",
                    &date[..4],
                    &date[4..6],
                    &date[6..8],
                    &time[..2],
                    &time[2..4]
                );
            }
        }
    }
    url.rsplit('/').next().unwrap_or("").to_string()
}

/// Compute time span string from a slice of satellite timestamps (same format as radar helper).
fn sat_time_span_str(timestamps: &[String]) -> String {
    if timestamps.len() < 2 {
        return String::new();
    }
    use chrono::NaiveDateTime;
    let parse = |s: &str| -> Option<NaiveDateTime> {
        let mut parts = s.split_whitespace();
        let date = parts.next()?;
        let time = parts.next()?;
        NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M").ok()
    };
    if let (Some(t0), Some(t1)) = (
        parse(timestamps.first().unwrap()),
        parse(timestamps.last().unwrap()),
    ) {
        let secs = (t1 - t0).num_seconds().abs();
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if h > 0 {
            format!(" | {h}h {m}m")
        } else {
            format!(" | {m}m")
        }
    } else {
        String::new()
    }
}

#[allow(clippy::too_many_arguments)]
fn fetch_sat_animation(
    state: Rc<RefCell<SatState>>,
    da: DrawingArea,
    status: Label,
    running: Rc<Cell<bool>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    sector: String,
    band: String,
    frame_count: usize,
    timeline: Scale,
    slider_updating: Rc<Cell<bool>>,
    anim_btn: Button,
) {
    anim_btn.set_sensitive(false);

    let progress: crate::runtime::ProgressSlot = Arc::new(Mutex::new(None));
    let stop_progress = crate::runtime::progress_poller(Arc::clone(&progress), status.clone());

    let progress_c = Arc::clone(&progress);
    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            let urls = goes::animation_urls(&client, &sector, &band, frame_count).await?;
            let total = urls.len();
            let mut frames: Vec<(String, Vec<u8>)> = Vec::new();
            for (i, url) in urls.iter().enumerate() {
                if let Ok(mut g) = progress_c.lock() {
                    *g = Some(format!("Fetching frame {}/{total}", i + 1));
                }
                let bytes = goes::fetch_image(&client, url).await?;
                frames.push((url.clone(), bytes));
            }
            Ok::<_, anyhow::Error>(frames)
        },
        move |result| {
            stop_progress.set(true);
            anim_btn.set_sensitive(true);
            match result {
                Ok(frames) => {
                    let pixbufs: Vec<Pixbuf> = frames
                        .iter()
                        .filter_map(|(_, b)| bytes_to_pixbuf(b))
                        .collect();
                    let timestamps: Vec<String> = frames
                        .iter()
                        .map(|(url, _)| sat_timestamp_from_url(url))
                        .collect();
                    if pixbufs.is_empty() {
                        status.set_text("Animation: no frames");
                        running.set(false);
                        return;
                    }
                    let n = pixbufs.len();
                    let span = sat_time_span_str(&timestamps);
                    {
                        let mut st = state.borrow_mut();
                        st.anim_frames = pixbufs;
                        st.anim_timestamps = timestamps;
                        st.anim_index = 0;
                        st.current_pixbuf = Some(st.anim_frames[0].clone());
                    }
                    // Configure timeline
                    slider_updating.set(true);
                    timeline.set_range(0.0, (n - 1) as f64);
                    timeline.set_value(0.0);
                    timeline.set_sensitive(true);
                    slider_updating.set(false);
                    status.set_text(&format!("Animating {n} frames{span}"));
                    start_sat_timer(
                        Rc::clone(&state),
                        da.clone(),
                        Rc::clone(&running),
                        Rc::clone(&timer),
                        timeline,
                        slider_updating,
                    );
                }
                Err(e) => {
                    status.set_text(&format!("Anim error: {e}"));
                    running.set(false);
                }
            }
        },
    );
}

fn start_sat_timer(
    state: Rc<RefCell<SatState>>,
    da: DrawingArea,
    running: Rc<Cell<bool>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    timeline: Scale,
    slider_updating: Rc<Cell<bool>>,
) {
    let id = glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
        if !running.get() {
            return glib::ControlFlow::Break;
        }
        let mut st = state.borrow_mut();
        if st.anim_frames.is_empty() {
            return glib::ControlFlow::Break;
        }
        st.anim_index = (st.anim_index + 1) % st.anim_frames.len();
        let i = st.anim_index;
        st.current_pixbuf = Some(st.anim_frames[i].clone());
        drop(st);
        slider_updating.set(true);
        timeline.set_value(i as f64);
        slider_updating.set(false);
        da.queue_draw();
        glib::ControlFlow::Continue
    });
    *timer.borrow_mut() = Some(id);
}
