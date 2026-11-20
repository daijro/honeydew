//! Global display preferences only. Never part of a song or an audio-engine edit.
use eframe::egui::{self, Color32, Id, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnvelopeDisplay {
    pub visible: bool,
    pub colors: [[u8; 3]; 2],
}
impl Default for EnvelopeDisplay {
    fn default() -> Self { Self { visible: true, colors: [[255, 213, 64], [80, 222, 255]] } }
}
impl EnvelopeDisplay {
    pub fn publish(self, ctx: &egui::Context) { ctx.data_mut(|d| d.insert_temp(Id::new("envelope-display"), self)); }
    pub fn current(ctx: &egui::Context) -> Self { ctx.data(|d| d.get_temp(Id::new("envelope-display"))).unwrap_or_default() }
    pub fn color(self, lane: usize) -> Color32 { let [r,g,b] = self.colors[lane]; Color32::from_rgb(r,g,b) }
}
pub fn open(ctx: &egui::Context) { ctx.data_mut(|d| d.insert_temp(Id::new("envelope-colors-open"), true)); }
fn parse_hex(text: &str) -> Option<[u8;3]> {
    let text = text.trim().trim_start_matches('#');
    if text.len() != 6 { return None; }
    let v = u32::from_str_radix(text,16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}
pub fn panel(ctx: &egui::Context, prefs: &mut EnvelopeDisplay) {
    let id = Id::new("envelope-colors-open");
    let mut open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if !open { return; }
    egui::Window::new("Edit · Envelope display").open(&mut open)
        .anchor(egui::Align2::RIGHT_TOP, vec2(-8.0, 36.0))
        .default_width(252.0).max_height((ctx.content_rect().height() - 72.0).max(100.0)).vscroll(true).resizable(false).collapsible(false).show(ctx, |ui| {
        ui.set_max_width(252.0);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, 3.0, Color32::from_rgb(30,36,46));
        for i in 0..64 {
            let x = r.left() + (i as f32 + 0.5) * r.width()/64.0;
            let h = 4.0 + ((i as f32 * 1.7).sin().abs()) * 17.0;
            p.line_segment([pos2(x,r.center().y-h),pos2(x,r.center().y+h)],Stroke::new(1.0,Color32::from_gray(110)));
        }
        for (k, ys) in [[0.8,0.3,0.5,0.15],[0.25,0.6,0.5,0.7]].iter().enumerate() {
            let pts = ys.iter().enumerate().map(|(i,y)| pos2(r.left()+i as f32*r.width()/3.0,r.top()+y*r.height())).collect();
            p.add(egui::Shape::line(pts,Stroke::new(2.0,prefs.color(k))));
        }
        ui.checkbox(&mut prefs.visible,"Show Playlist envelopes");
        for (k,label) in ["Volume (VOL) Color", "Panning (PAN) Color"].iter().enumerate() {
            ui.label(*label);
            ui.horizontal_wrapped(|ui| {
                // Deliberately no black preset. HEX accepts any user-specified RGB color.
                for color in [[255,213,64],[80,222,255],[255,255,255],[80,140,255],[255,100,130],[130,235,150],[200,130,255]] {
                    let [r,g,b] = color;
                    if ui.add(egui::Button::new("  ").fill(Color32::from_rgb(r,g,b)).min_size(vec2(26.0,26.0))).clicked() { prefs.colors[k] = color; }
                }
            });
            let key = Id::new(("envelope-hex",k));
            let [r,g,b] = prefs.colors[k];
            let mut hex = ctx.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| format!("#{r:02X}{g:02X}{b:02X}"));
            let response = ui.add(egui::TextEdit::singleline(&mut hex).hint_text("#RRGGBB").desired_width(120.0));
            if response.changed() { if let Some(rgb) = parse_hex(&hex) { prefs.colors[k] = rgb; } }
            if !response.has_focus() { let [r,g,b] = prefs.colors[k]; hex = format!("#{r:02X}{g:02X}{b:02X}"); }
            ctx.data_mut(|d| d.insert_temp(key,hex));
        }
        if ui.button("Reset display colors").clicked() { prefs.colors = EnvelopeDisplay::default().colors; }
    });
    ctx.data_mut(|d| d.insert_temp(id,open));
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn clip_envelope_display_colors_roundtrip_and_hex_are_independent() {
        let mut p = EnvelopeDisplay::default(); let pan=p.colors[1];
        p.colors[0]=parse_hex("#2488FF").unwrap(); assert_eq!(p.colors[1],pan);
        assert_eq!(parse_hex("FFFFFF"),Some([255;3])); assert_eq!(parse_hex("xyz"),None);
        p.visible=false;
        assert_eq!(serde_json::from_str::<EnvelopeDisplay>(&serde_json::to_string(&p).unwrap()).unwrap(),p);
    }
}
