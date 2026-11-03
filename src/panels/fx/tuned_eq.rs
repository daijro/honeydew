//! The Tuned EQ panel (ID 30), UIHeight 10 U.

use crate::panels::build::*;

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        hstack([
            v_slider(1, "65 Hz").p(1).label_fn(band::<0>),
            v_slider(2, "130 Hz").p(2).label_fn(band::<1>),
            v_slider(3, "261 Hz").p(3).label_fn(band::<2>),
            v_slider(4, "523 Hz").p(4).label_fn(band::<3>),
            v_slider(5, "1.0 kHz").p(5).label_fn(band::<4>),
            v_slider(6, "2.1 kHz").p(6).label_fn(band::<5>),
            v_slider(7, "4.2 kHz").p(7).label_fn(band::<6>),
            v_slider(8, "8.4 kHz").p(8).label_fn(band::<7>),
            v_slider(9, "BW").p(10).h(Size::Percent(70.0)),
        ]),
        list(
            10,
            "C · C# · D · D# · E · F · F# · G · G# · A · A# · B",
            Bar::Bottom,
        )
        .p(9)
        .h(Size::U(1.25)),
    ])))
}

/// Band `K`'s label: the frequency of the p9 key in octave `K` + 2, whole Hz below 1 kHz and one decimal in kHz
/// above.
fn band<const K: i32>(params: &[f32]) -> String {
    // The key the DSP tunes to: the param's own law.
    let tuning = crate::params::module_spec(30).unwrap().params[9].kind;
    let key = tuning.item(params.get(9).copied().unwrap_or(0.0)) as f64;
    let hz = 440.0 * 2f64.powf((key - 9.0) / 12.0 + f64::from(K - 2));
    if hz < 1000.0 {
        format!("{} Hz", hz.trunc())
    } else {
        format!("{:.1} kHz", hz / 1000.0)
    }
}

/// The band rows of the key `labels`.
#[cfg(test)]
const fn bands(labels: [&'static str; 8]) -> [Row; 10] {
    const X: [[f32; 2]; 8] = [
        [0.0, 1.55],
        [1.55, 1.56],
        [3.11, 1.55],
        [4.66, 1.56],
        [6.22, 1.55],
        [7.77, 1.56],
        [9.33, 1.55],
        [10.88, 1.56],
    ];
    let mut rows = [row(9, Widget::VSlider, "BW", [12.44, 1.01, 1.56, 4.71])
        .p(10)
        .h(Size::Percent(70.0)); 10];
    let mut k = 0;
    while k < 8 {
        let [x, w] = X[k];
        rows[k] = row(k as u32 + 1, Widget::VSlider, labels[k], [x, 0.0, w, 6.74]).p(k + 1);
        k += 1;
    }
    rows[9] = row(
        10,
        Widget::List(Bar::Bottom),
        "C · C# · D · D# · E · F · F# · G · G# · A · A# · B",
        [0.0, 6.74, 14.0, 1.25],
    )
    .p(9)
    .h(Size::U(1.25));
    rows
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 7.99],
    settings: &[],
    cases: &[
        Case {
            name: "C",
            rows: &bands([
                "65 Hz", "130 Hz", "261 Hz", "523 Hz", "1.0 kHz", "2.1 kHz", "4.2 kHz", "8.4 kHz",
            ]),
            ..Case::DEFAULT
        },
        Case {
            name: "C#",
            params: &[(9, crate::params::menu_value(1, 12))],
            rows: &bands([
                "69 Hz", "138 Hz", "277 Hz", "554 Hz", "1.1 kHz", "2.2 kHz", "4.4 kHz", "8.9 kHz",
            ]),
            ..Case::DEFAULT
        },
        Case {
            name: "A",
            params: &[(9, crate::params::menu_value(9, 12))],
            rows: &bands([
                "110 Hz", "220 Hz", "440 Hz", "880 Hz", "1.8 kHz", "3.5 kHz", "7.0 kHz", "14.1 kHz",
            ]),
            ..Case::DEFAULT
        },
        Case {
            name: "B",
            params: &[(9, crate::params::menu_value(11, 12))],
            rows: &bands([
                "123 Hz", "246 Hz", "493 Hz", "987 Hz", "2.0 kHz", "4.0 kHz", "7.9 kHz", "15.8 kHz",
            ]),
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::menu_value;

    #[test]
    fn the_band_labels_read_the_key_by_its_param_law() {
        // Tuning is trunc(fma_f64(v, 11, f32(0.001))): 0.5 is F, as the DSP tunes, not the nearest
        // item F#.
        let at = |v: f32| {
            let mut p = [0.0; 11];
            p[9] = v;
            band::<2>(&p)
        };
        assert_eq!(at(0.5), at(menu_value(5, 12)));
    }
}
