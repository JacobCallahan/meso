/*
 * SPC (Storm Prediction Center) viewer pane.
 *
 * Displays:
 *   - Convective outlooks (Day 1/2/3) as PNG images
 *   - Today's storm reports (tornado, hail, wind) as a scrollable list
 *   - 📊 Meso: SPC Mesoanalysis images (toggle via button in toolbar)
 */

use gdk_pixbuf::Pixbuf;
use gtk4::prelude::*;
use gtk4::{
    Box as GBox, Button, DropDown, Label, ListBox, ListBoxRow, Orientation, Paned, PolicyType,
    ScrolledWindow, Separator, StringList, ToggleButton,
};

use std::cell::RefCell;
use std::rc::Rc;

use crate::config::Config;
use crate::runtime;
use crate::ui::enable_status_copy;
use meso_data::mesoanalysis::{self, MESO_PRODUCTS};
use meso_data::spc::{self, ReportType, StormReport};

// ── State ─────────────────────────────────────────────────────────────────────

struct SpcState {
    current_day: u8,
    current_pixbuf: Option<Pixbuf>,
    meso_mode: bool,
    meso_product: String,
    zoom: f64,
    pan_x: f64,
    pan_y: f64,
}

// ── Public builder ────────────────────────────────────────────────────────────

pub fn build_spc_pane(shared_config: Rc<RefCell<Config>>) -> GBox {
    let vbox = GBox::new(Orientation::Vertical, 0);

    // Toolbar
    let toolbar = GBox::new(Orientation::Horizontal, 4);
    toolbar.set_margin_start(4);
    toolbar.set_margin_end(4);
    toolbar.set_margin_top(4);
    toolbar.set_margin_bottom(4);

    // Outlook controls (Day 1/2/3 + refresh)
    let day1_btn = Button::with_label("Day 1");
    let day2_btn = Button::with_label("Day 2");
    let day3_btn = Button::with_label("Day 3");
    let refresh_btn = Button::with_label("⟳");

    // Mesoanalysis controls
    let meso_refresh_btn = Button::with_label("⟳");
    let meso_model = StringList::new(
        &MESO_PRODUCTS.iter().map(|p| p.label).collect::<Vec<_>>(),
    );
    let meso_combo = DropDown::new(Some(meso_model), gtk4::Expression::NONE);
    meso_combo.set_selected(0);
    meso_combo.set_hexpand(false);

    // Initially hide mesoanalysis controls
    meso_combo.set_visible(false);
    meso_refresh_btn.set_visible(false);

    // Meso toggle button (stays always visible)
    let meso_toggle = ToggleButton::with_label("📊 Meso");

    let status = Label::new(Some("Loading..."));
    status.set_hexpand(true);
    status.set_halign(gtk4::Align::Start);
    status.set_margin_start(8);
    enable_status_copy(&status);

    toolbar.append(&day1_btn);
    toolbar.append(&day2_btn);
    toolbar.append(&day3_btn);
    toolbar.append(&refresh_btn);
    toolbar.append(&meso_combo);
    toolbar.append(&meso_refresh_btn);
    toolbar.append(&meso_toggle);
    toolbar.append(&status);
    vbox.append(&toolbar);

    // Horizontal split: left = image, right = storm reports list
    let paned = Paned::new(Orientation::Horizontal);
    paned.set_vexpand(true);

    // Restore or auto-fit position (left child = ~75% of width)
    let saved_pos = shared_config.borrow().spc_pane_position;
    if saved_pos > 0 {
        paned.set_position(saved_pos);
    } else {
        let p_clone = paned.clone();
        paned.connect_realize(move |_| {
            let w = p_clone.width();
            if w > 10 {
                p_clone.set_position((w as f64 * 0.72) as i32);
            }
        });
    }

    // Save position changes back to config
    {
        let cfg = Rc::clone(&shared_config);
        paned.connect_position_notify(move |p| {
            cfg.borrow_mut().spc_pane_position = p.position();
        });
    }

    // Left: image in a drawing area
    let drawing_area = gtk4::DrawingArea::new();
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);
    let left_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .child(&drawing_area)
        .build();
    paned.set_start_child(Some(&left_scroll));

    // Right: storm report list
    let report_list = ListBox::new();
    report_list.set_selection_mode(gtk4::SelectionMode::None);
    let right_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .child(&report_list)
        .build();
    right_scroll.set_size_request(320, -1);
    paned.set_end_child(Some(&right_scroll));

    vbox.append(&paned);

    // State
    let state = Rc::new(RefCell::new(SpcState {
        current_day: 1,
        current_pixbuf: None,
        meso_mode: false,
        meso_product: MESO_PRODUCTS[0].id.to_string(),
        zoom: 1.0,
        pan_x: 0.0,
        pan_y: 0.0,
    }));

    // Draw callback
    {
        let state_d = Rc::clone(&state);
        drawing_area.set_draw_func(move |_da, ctx, w, h| {
            // Keep a white backdrop so transparent meso images stay legible.
            ctx.set_source_rgb(1.0, 1.0, 1.0);
            let _ = ctx.paint();
            let st = state_d.borrow();
            if let Some(pb) = &st.current_pixbuf {
                let img_w = pb.width() as f64;
                let img_h = pb.height() as f64;
                let fit = (w as f64 / img_w).min(h as f64 / img_h);
                let scale = fit * st.zoom;
                let off_x = (w as f64 - img_w * scale) / 2.0 - st.pan_x * scale;
                let off_y = (h as f64 - img_h * scale) / 2.0 - st.pan_y * scale;
                ctx.translate(off_x, off_y);
                ctx.scale(scale, scale);
                ctx.set_source_pixbuf(pb, 0.0, 0.0);
                let _ = ctx.paint();
            }
        });
    }

    // Scroll to zoom
    {
        let state_z = Rc::clone(&state);
        let da_z = drawing_area.clone();
        let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(move |_, _dx, dy| {
            let factor = if dy < 0.0 { 1.15 } else { 1.0 / 1.15 };
            zoom_around(&mut state_z.borrow_mut(), factor, &da_z);
            da_z.queue_draw();
            glib::Propagation::Stop
        });
        drawing_area.add_controller(scroll);
    }

    // Drag to pan
    {
        let state_p = Rc::clone(&state);
        let da_p = drawing_area.clone();
        let pan_origin: Rc<RefCell<(f64, f64)>> = Rc::new(RefCell::new((0.0, 0.0)));
        let drag = gtk4::GestureDrag::new();
        {
            let po = Rc::clone(&pan_origin);
            let st_c = Rc::clone(&state_p);
            drag.connect_drag_begin(move |_, _x, _y| {
                let st = st_c.borrow();
                *po.borrow_mut() = (st.pan_x, st.pan_y);
            });
        }
        {
            let po = Rc::clone(&pan_origin);
            let st_c = Rc::clone(&state_p);
            let da_c = da_p.clone();
            drag.connect_drag_update(move |_, ox, oy| {
                let (origin_x, origin_y) = *po.borrow();
                let (img_w, img_h) = st_c
                    .borrow()
                    .current_pixbuf
                    .as_ref()
                    .map(|pb| (pb.width() as f64, pb.height() as f64))
                    .unwrap_or((1.0, 1.0));
                let fit = (da_c.width() as f64 / img_w).min(da_c.height() as f64 / img_h);
                let scale = fit * st_c.borrow().zoom;
                if scale > 0.0 {
                    let mut st = st_c.borrow_mut();
                    st.pan_x = origin_x - ox / scale;
                    st.pan_y = origin_y - oy / scale;
                    drop(st);
                    da_c.queue_draw();
                }
            });
        }
        drawing_area.add_controller(drag);
    }

    // Wire outlook buttons
    let outlook_btns = vec![
        day1_btn.clone(),
        day2_btn.clone(),
        day3_btn.clone(),
        refresh_btn.clone(),
    ];
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let rl_c = report_list.clone();
        let btns = outlook_btns.clone();
        day1_btn.connect_clicked(move |_| {
            state_c.borrow_mut().current_day = 1;
            load_outlook(
                Rc::clone(&state_c),
                da_c.clone(),
                st_c.clone(),
                rl_c.clone(),
                btns.clone(),
            );
        });
    }
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let rl_c = report_list.clone();
        let btns = outlook_btns.clone();
        day2_btn.connect_clicked(move |_| {
            state_c.borrow_mut().current_day = 2;
            load_outlook(
                Rc::clone(&state_c),
                da_c.clone(),
                st_c.clone(),
                rl_c.clone(),
                btns.clone(),
            );
        });
    }
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let rl_c = report_list.clone();
        let btns = outlook_btns.clone();
        day3_btn.connect_clicked(move |_| {
            state_c.borrow_mut().current_day = 3;
            load_outlook(
                Rc::clone(&state_c),
                da_c.clone(),
                st_c.clone(),
                rl_c.clone(),
                btns.clone(),
            );
        });
    }
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let rl_c = report_list.clone();
        let btns = outlook_btns.clone();
        refresh_btn.connect_clicked(move |_| {
            load_outlook(
                Rc::clone(&state_c),
                da_c.clone(),
                st_c.clone(),
                rl_c.clone(),
                btns.clone(),
            );
        });
    }

    // Wire mesoanalysis controls
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let combo_c = meso_combo.clone();
        meso_combo.connect_selected_notify(move |combo| {
            let idx = combo.selected() as usize;
            if let Some(prod) = MESO_PRODUCTS.get(idx) {
                state_c.borrow_mut().meso_product = prod.id.to_string();
                load_meso(
                    Rc::clone(&state_c),
                    da_c.clone(),
                    st_c.clone(),
                    combo_c.clone(),
                );
            }
        });
    }
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let combo_c = meso_combo.clone();
        meso_refresh_btn.connect_clicked(move |_| {
            load_meso(
                Rc::clone(&state_c),
                da_c.clone(),
                st_c.clone(),
                combo_c.clone(),
            );
        });
    }

    // Meso toggle → show/hide controls + load
    {
        let state_c = Rc::clone(&state);
        let da_c = drawing_area.clone();
        let st_c = status.clone();
        let rl_c = report_list.clone();
        let combo_c = meso_combo.clone();
        let d1 = day1_btn.clone();
        let d2 = day2_btn.clone();
        let d3 = day3_btn.clone();
        let rf = refresh_btn.clone();
        let mrc = meso_refresh_btn.clone();
        let btns_out = outlook_btns.clone();
        meso_toggle.connect_toggled(move |btn| {
            let on = btn.is_active();
            state_c.borrow_mut().meso_mode = on;
            d1.set_visible(!on);
            d2.set_visible(!on);
            d3.set_visible(!on);
            rf.set_visible(!on);
            combo_c.set_visible(on);
            mrc.set_visible(on);
            if on {
                load_meso(
                    Rc::clone(&state_c),
                    da_c.clone(),
                    st_c.clone(),
                    combo_c.clone(),
                );
            } else {
                // Reload the current outlook
                load_outlook(
                    Rc::clone(&state_c),
                    da_c.clone(),
                    st_c.clone(),
                    rl_c.clone(),
                    btns_out.clone(),
                );
            }
        });
    }

    // Initial load
    {
        let btns = outlook_btns.clone();
        load_outlook(
            Rc::clone(&state),
            drawing_area.clone(),
            status.clone(),
            report_list.clone(),
            btns,
        );
    }

    vbox
}

// ── Mesoanalysis loading ──────────────────────────────────────────────────────

fn load_meso(
    state: Rc<RefCell<SpcState>>,
    da: gtk4::DrawingArea,
    status: Label,
    combo: DropDown,
) {
    let product_id = state.borrow().meso_product.clone();
    let product_label = MESO_PRODUCTS
        .iter()
        .find(|p| p.id == product_id)
        .map(|p| p.label)
        .unwrap_or(&product_id);
    status.set_text(&format!("Fetching meso: {product_label}…"));
    combo.set_sensitive(false);

    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            mesoanalysis::fetch_meso_image(&client, &product_id).await
        },
        move |result| {
            combo.set_sensitive(true);
            match result {
                Ok(bytes) => {
                    if let Some(pb) = bytes_to_pixbuf(&bytes) {
                        let mut st = state.borrow_mut();
                        st.current_pixbuf = Some(pb);
                        st.zoom = 1.0;
                        st.pan_x = 0.0;
                        st.pan_y = 0.0;
                        drop(st);
                        da.queue_draw();
                        let label = MESO_PRODUCTS
                            .iter()
                            .find(|p| p.id == state.borrow().meso_product)
                            .map(|p| p.label)
                            .unwrap_or("Mesoanalysis");
                        status.set_text(&format!("Meso: {label}"));
                    } else {
                        status.set_text("Failed to decode mesoanalysis image");
                    }
                }
                Err(e) => status.set_text(&format!("Meso error: {e}")),
            }
        },
    );
}

// ── Outlook loading ───────────────────────────────────────────────────────────

fn load_outlook(
    state: Rc<RefCell<SpcState>>,
    da: gtk4::DrawingArea,
    status: Label,
    report_list: ListBox,
    btns: Vec<Button>,
) {
    let day = state.borrow().current_day;
    for b in &btns {
        b.set_sensitive(false);
    }
    status.set_text(&format!("Loading Day {day} outlook + storm reports..."));

    runtime::spawn(
        async move {
            let client = meso_data::http::wx_client();
            let img = spc::fetch_outlook_image(&client, day).await?;
            let reports = spc::fetch_storm_reports(&client).await.unwrap_or_default();
            Ok::<_, anyhow::Error>((img, reports))
        },
        move |result| {
            for b in &btns {
                b.set_sensitive(true);
            }
            match result {
                Ok((img_bytes, reports)) => {
                    if let Some(pb) = bytes_to_pixbuf(&img_bytes) {
                        let mut st = state.borrow_mut();
                        st.current_pixbuf = Some(pb);
                        st.zoom = 1.0;
                        st.pan_x = 0.0;
                        st.pan_y = 0.0;
                        drop(st);
                        da.queue_draw();
                    }
                    populate_report_list(&report_list, &reports);
                    let day = state.borrow().current_day;
                    status.set_text(&format!(
                        "Day {day} outlook — {} storm reports",
                        reports.len()
                    ));
                }
                Err(e) => status.set_text(&format!("Error: {e}")),
            }
        },
    );
}

fn populate_report_list(list: &ListBox, reports: &[StormReport]) {
    // Remove all existing rows
    while let Some(row) = list.first_child() {
        list.remove(&row);
    }

    // Header
    let hdr = ListBoxRow::new();
    let hdr_lbl = Label::new(Some("  Time   Type   Mag   Location"));
    hdr_lbl.set_xalign(0.0);
    hdr_lbl.set_margin_start(8);
    hdr_lbl.set_margin_top(4);
    hdr_lbl.set_margin_bottom(4);
    let hdr_attrs = pango_bold_attrs();
    hdr_lbl.set_attributes(Some(&hdr_attrs));
    hdr.set_child(Some(&hdr_lbl));
    list.append(&hdr);

    let sep = Separator::new(Orientation::Horizontal);
    list.append(&sep);

    for report in reports {
        let row = ListBoxRow::new();
        let row_box = GBox::new(Orientation::Horizontal, 6);
        row_box.set_margin_start(8);
        row_box.set_margin_end(8);
        row_box.set_margin_top(3);
        row_box.set_margin_bottom(3);

        // Color indicator
        let indicator = gtk4::DrawingArea::new();
        indicator.set_size_request(8, 20);
        let color = match report.report_type {
            ReportType::Tornado => (0.8, 0.0, 0.0),
            ReportType::Hail => (0.0, 0.6, 0.0),
            ReportType::Wind => (0.2, 0.4, 0.8),
        };
        indicator.set_draw_func(move |_, ctx, _w, h| {
            ctx.set_source_rgb(color.0, color.1, color.2);
            ctx.rectangle(0.0, 0.0, 8.0, h as f64);
            let _ = ctx.fill();
        });
        row_box.append(&indicator);

        let text = format!(
            "{} | {} | {} | {}, {}",
            report.time,
            report.report_type.label(),
            if report.magnitude.is_empty() {
                "—"
            } else {
                &report.magnitude
            },
            report.location,
            report.state,
        );
        let lbl = Label::new(Some(&text));
        lbl.set_xalign(0.0);
        lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        lbl.set_hexpand(true);

        // Show comments in tooltip
        if !report.comments.is_empty() {
            lbl.set_tooltip_text(Some(&report.comments));
        }

        row_box.append(&lbl);
        row.set_child(Some(&row_box));
        list.append(&row);
    }

    if reports.is_empty() {
        let empty = ListBoxRow::new();
        let lbl = Label::new(Some("No storm reports today"));
        lbl.set_margin_top(12);
        lbl.set_margin_bottom(12);
        empty.set_child(Some(&lbl));
        list.append(&empty);
    }
}

fn pango_bold_attrs() -> gtk4::pango::AttrList {
    let list = gtk4::pango::AttrList::new();
    list.insert(gtk4::pango::AttrInt::new_weight(gtk4::pango::Weight::Bold));
    list
}

fn bytes_to_pixbuf(bytes: &[u8]) -> Option<Pixbuf> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    loader.pixbuf()
}

fn zoom_around(st: &mut SpcState, factor: f64, da: &gtk4::DrawingArea) {
    if let Some(pb) = &st.current_pixbuf {
        let img_w = pb.width() as f64;
        let img_h = pb.height() as f64;
        let widget_w = da.width() as f64;
        let widget_h = da.height() as f64;
        let fit = (widget_w / img_w).min(widget_h / img_h);
        let total = fit * st.zoom;
        let cx = widget_w / 2.0;
        let cy = widget_h / 2.0;
        let img_x = (cx - widget_w / 2.0) / total + img_w / 2.0 + st.pan_x;
        let img_y = (cy - widget_h / 2.0) / total + img_h / 2.0 + st.pan_y;
        st.zoom = (st.zoom * factor).clamp(0.1, 20.0);
        let new_total = fit * st.zoom;
        st.pan_x = img_x - img_w / 2.0 - (cx - widget_w / 2.0) / new_total;
        st.pan_y = img_y - img_h / 2.0 - (cy - widget_h / 2.0) / new_total;
    } else {
        st.zoom = (st.zoom * factor).clamp(0.1, 20.0);
    }
}
