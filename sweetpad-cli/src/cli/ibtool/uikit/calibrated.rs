//! Generic RGB/Gray (gamma 1.8) to the UIKit archive's sRGB transfer space.
use moxcms::{ColorProfile, Layout, ToneReprCurve, TransformOptions, Xyzd, curve_from_gamma};

pub(super) fn gray(white: f64) -> Result<f64, String> {
    let source = ColorProfile::new_gray_with_gamma(1.8);
    let mut target = ColorProfile::new_gray_with_gamma(2.2);
    target.gray_trc = Some(srgb_curve());
    let transform = source
        .create_transform_f32(
            Layout::Gray,
            &target,
            Layout::Gray,
            TransformOptions::default(),
        )
        .map_err(|e| e.to_string())?;
    let mut result = [0.0];
    #[allow(clippy::cast_possible_truncation)]
    transform
        .transform(&[white as f32], &mut result)
        .map_err(|e| e.to_string())?;
    Ok(f64::from(result[0]))
}

fn srgb_curve() -> ToneReprCurve {
    ToneReprCurve::Parametric(vec![2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045])
}

pub(super) fn rgb(components: [f64; 3]) -> Result<[f64; 3], String> {
    let mut source = ColorProfile::new_srgb();
    source.cicp = None;
    // D50-adapted colorants of the standard Generic RGB color space.
    source.red_colorant = Xyzd {
        x: 0.454_299_926_757_812_5,
        y: 0.241_912_841_796_875,
        z: 0.014_892_578_125,
    };
    source.green_colorant = Xyzd {
        x: 0.353_347_778_320_312_5,
        y: 0.673_629_760_742_187_5,
        z: 0.090_637_207_031_25,
    };
    source.blue_colorant = Xyzd {
        x: 0.156_646_728_515_625,
        y: 0.084_457_397_460_937_5,
        z: 0.719_573_974_609_375,
    };
    source.red_trc = Some(curve_from_gamma(1.8));
    source.green_trc = source.red_trc.clone();
    source.blue_trc = source.red_trc.clone();
    let transform = source
        .create_transform_f32(
            Layout::Rgb,
            &ColorProfile::new_srgb(),
            Layout::Rgb,
            TransformOptions::default(),
        )
        .map_err(|e| e.to_string())?;
    let mut result = [0.0; 3];
    #[allow(clippy::cast_possible_truncation)]
    transform
        .transform(&components.map(|v| v as f32), &mut result)
        .map_err(|e| e.to_string())?;
    Ok(result.map(f64::from))
}
