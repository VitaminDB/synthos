use syngui::prelude::*;
use syngui::widgets::Reactive;

pub fn fmt_mmss(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

pub fn fmt_hhmmss(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{:02}:{:02}:{:02}", h, m, sec)
    } else {
        format!("{:02}:{:02}", m, sec)
    }
}

pub fn node_timecode(
    elapsed: RwSignal<f64>,
    total: RwSignal<Option<f64>>,
) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let cur = elapsed.get();
        let dur_opt = total.get();
        let txt = match dur_opt {
            Some(dur) => format!("{} / {}", fmt_mmss(cur), fmt_mmss(dur)),
            None => format!("{} / --:--", fmt_mmss(cur)),
        };
        vec![Box::new(Text::new(txt).class("node-timecode")) as Box<dyn Widget>]
    }))
}
