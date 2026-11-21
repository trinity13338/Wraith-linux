//! A small message window (MessageBox on Windows): a question with Yes/No, or a notice with OK.

use super::theme::{self, semibold};
use super::widgets::{self as w, Kind};
use egui::{FontId, Frame, Margin, RichText};
use std::sync::{Arc, Mutex};

struct Dialog { text: String, question: bool, answer: Arc<Mutex<bool>>, pal: theme::Palette }

impl eframe::App for Dialog {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let p = self.pal;
        egui::CentralPanel::default().frame(Frame::new().fill(p.surface).inner_margin(Margin::same(24))).show(ctx, |ui| {
            ui.label(RichText::new("Wraith").font(FontId::new(20.0, semibold())));
            ui.add_space(8.0);
            ui.label(RichText::new(&self.text).color(p.text));
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.question {
                            if w::button(ui, &p, Kind::Filled, None, crate::tr!("Да", "Yes")).clicked() { *self.answer.lock().unwrap() = true; ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                            if w::button(ui, &p, Kind::Text, None, crate::tr!("Нет", "No")).clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                        } else if w::button(ui, &p, Kind::Filled, None, "OK").clicked() { *self.answer.lock().unwrap() = true; ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                    });
                });
            });
        });
    }
}

/// Shows the message and waits. Returns true for "Yes" (or OK); false when declined or closed.
pub fn ask(text: &str, question: bool) -> bool {
    let answer = Arc::new(Mutex::new(false));
    let a2 = answer.clone(); let text = text.to_string();
    let icon = egui::IconData { rgba: super::icon::rgba(64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title(super::TITLE).with_app_id(super::APP_ID).with_inner_size([460.0, 220.0]).with_resizable(false).with_icon(Arc::new(icon)),
        centered: true, ..Default::default()
    };
    let r = eframe::run_native(super::TITLE, options, Box::new(move |cc| {
        theme::install_fonts(&cc.egui_ctx);
        let pal = theme::palette(&crate::store::try_theme()); theme::apply(&cc.egui_ctx, &pal);
        Ok(Box::new(Dialog { text, question, answer: a2, pal }))
    }));
    if let Err(e) = r { eprintln!("Wraith: {text_err}", text_err = e); }
    let v = *answer.lock().unwrap(); v
}
