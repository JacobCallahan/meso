/*
 * NWS text products viewer pane.
 *
 * Fetches NWS text products (AFD, HWO, ZFP, LSR, etc.) via api.weather.gov
 * for a given WFO (Weather Forecast Office).
 */

use gtk4::prelude::*;
use gtk4::{
    Box as GBox, DropDown, Entry, Label, Orientation, PolicyType, ScrolledWindow, StringList,
    TextView, WrapMode,
};

use std::cell::RefCell;
use std::rc::Rc;

use crate::runtime;
use crate::ui::enable_status_copy;
use meso_data::text_products::{self, PRODUCT_TYPES};

// ── State ─────────────────────────────────────────────────────────────────────

struct TextState {
    wfo: String,
    product_type: String,
}

// ── Public builder ────────────────────────────────────────────────────────────

pub fn build_text_pane(default_wfo: &str, radar_site: &str) -> GBox {
    let vbox = GBox::new(Orientation::Vertical, 0);

    // Toolbar
    let toolbar = GBox::new(Orientation::Horizontal, 4);
    toolbar.set_margin_start(4);
    toolbar.set_margin_end(4);
    toolbar.set_margin_top(4);
    toolbar.set_margin_bottom(4);

    let wfo_lbl = Label::new(Some("WFO:"));
    let wfo_entry = Entry::new();
    wfo_entry.set_text(default_wfo);
    wfo_entry.set_max_length(4);
    wfo_entry.set_width_chars(5);

    let prod_displays: Vec<String> = PRODUCT_TYPES
        .iter()
        .map(|(code, label)| format!("{code} — {label}"))
        .collect();
    let prod_ids: Rc<Vec<&'static str>> =
        Rc::new(PRODUCT_TYPES.iter().map(|(code, _)| *code).collect());
    let prod_model = StringList::new(&prod_displays.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    let prod_combo = DropDown::new(Some(prod_model), gtk4::Expression::NONE);
    prod_combo.set_selected(0); // default to AFD

    let status = Label::new(Some("Ready"));
    status.set_hexpand(true);
    status.set_halign(gtk4::Align::Start);
    status.set_margin_start(8);
    enable_status_copy(&status);

    toolbar.append(&wfo_lbl);
    toolbar.append(&wfo_entry);
    toolbar.append(&prod_combo);
    toolbar.append(&status);
    vbox.append(&toolbar);

    // Main text area
    let text_view = TextView::new();
    text_view.set_editable(false);
    text_view.set_wrap_mode(WrapMode::Word);
    text_view.set_monospace(true);
    text_view.set_left_margin(8);
    text_view.set_right_margin(8);
    text_view.set_top_margin(8);
    text_view.set_bottom_margin(8);

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .child(&text_view)
        .build();
    scroll.set_vexpand(true);
    vbox.append(&scroll);

    // State
    let state = Rc::new(RefCell::new(TextState {
        wfo: default_wfo.to_string(),
        product_type: "AFD".to_string(),
    }));

    // Wire product combo
    {
        let state_c = Rc::clone(&state);
        let prod_ids_c = Rc::clone(&prod_ids);
        prod_combo.connect_selected_notify(move |combo| {
            let idx = combo.selected() as usize;
            if let Some(&id) = prod_ids_c.get(idx) {
                state_c.borrow_mut().product_type = id.to_string();
            }
        });
    }

    // Wire fetch button (also triggered by Enter in WFO entry)
    let radar_site_for_wfo = radar_site.to_string();
    let do_fetch: Rc<dyn Fn()> = {
        let state_c = Rc::clone(&state);
        let tv_c = text_view.clone();
        let st_c = status.clone();
        let wfo_entry_c = wfo_entry.clone();
        let radar_site_c = radar_site_for_wfo.clone();
        Rc::new(move || {
            let wfo_input = wfo_entry_c.text().trim().to_uppercase();
            if wfo_input.is_empty() {
                st_c.set_text("Enter a WFO code");
                return;
            }
            let product_type = state_c.borrow().product_type.clone();
            state_c.borrow_mut().wfo = wfo_input.clone();

            st_c.set_text(&format!("Fetching {product_type} for {wfo_input}..."));

            let st_c2 = st_c.clone();
            let tv_c2 = tv_c.clone();
            let wfo_entry_c2 = wfo_entry_c.clone();
            let radar_site = radar_site_c.clone();

            runtime::spawn(
                async move {
                    let client = meso_data::http::wx_client();
                    let mut wfo = wfo_input.clone();
                    if let Ok(resolved) =
                        text_products::resolve_wfo_from_radar_site(&client, &radar_site).await
                    {
                        // If the current WFO is just the radar site code (e.g. RAX/KRAX),
                        // switch to true CWA office code (e.g. RAH).
                        let input_norm = wfo_input
                            .trim_start_matches('K')
                            .trim_start_matches('P')
                            .trim_start_matches('T')
                            .to_string();
                        let radar_norm = radar_site
                            .trim()
                            .to_uppercase()
                            .trim_start_matches('K')
                            .trim_start_matches('P')
                            .trim_start_matches('T')
                            .to_string();
                        if input_norm == radar_norm {
                            wfo = resolved;
                        }
                    }
                    let product =
                        text_products::fetch_latest_text(&client, &product_type, &wfo).await?;
                    Ok::<_, anyhow::Error>((product, wfo))
                },
                move |result| {
                    match result {
                        Ok((product, resolved_wfo)) => {
                            if wfo_entry_c2.text().trim().to_uppercase() != resolved_wfo {
                                wfo_entry_c2.set_text(&resolved_wfo);
                            }
                            let header = format!(
                                "{} — {} — {}\n{}\n",
                                product.product_code,
                                product.wfo,
                                product.issuance_time,
                                "─".repeat(60),
                            );
                            let full = format!("{}{}", header, product.text);
                            tv_c2.buffer().set_text(&full);
                            // Scroll to top
                            tv_c2.scroll_to_iter(
                                &mut tv_c2.buffer().start_iter(),
                                0.0,
                                false,
                                0.0,
                                0.0,
                            );
                            st_c2.set_text(&format!(
                                "{} — {} ({})",
                                product.product_code, product.wfo, product.issuance_time,
                            ));
                        }
                        Err(e) => {
                            st_c2.set_text(&format!("Error: {e}"));
                            tv_c2
                                .buffer()
                                .set_text(&format!("Error fetching product:\n{e}"));
                        }
                    }
                },
            );
        })
    };

    {
        let do_fetch_c = do_fetch.clone();
        prod_combo.connect_selected_notify(move |_| do_fetch_c());
    }
    {
        let do_fetch_c = do_fetch.clone();
        wfo_entry.connect_activate(move |_| do_fetch_c());
    }

    // Auto-fetch when pane is built.
    {
        let do_fetch_c = do_fetch.clone();
        gtk4::glib::idle_add_local_once(move || do_fetch_c());
    }

    vbox
}
