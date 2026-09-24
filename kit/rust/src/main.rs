use vector_rebuild::{advanced_parameters, preset, AdvancedSettings, Stage};

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("inspect") => {
            println!("Recovered settings toolkit. Run kit/app for raster-to-SVG conversion and desktop preview.");
            for code in 0..10 {
                println!("preset {code}: {} named parameters", preset(code)?.len());
            }
        }
        Some("derive") => {
            let mut s = AdvancedSettings::default();
            let stage = match args.get(1).map(String::as_str) {
                Some("palette") => Stage::Palette,
                Some("segment") => Stage::Segmentation,
                Some("smooth") => Stage::Smoothing,
                Some("fit") => Stage::Fitting,
                _ => return Err(
                    "derive requires palette|segment|smooth|fit [segmentation smoothness curve]"
                        .into(),
                ),
            };
            if args.len() != 2 && args.len() != 5 {
                return Err("Supply either no slider values or all three".into());
            }
            if args.len() == 5 {
                let parse = |i: usize| {
                    args[i]
                        .parse::<i32>()
                        .map_err(|_| "Expected integer slider value".to_owned())
                };
                s.segmentation_complexity = parse(2)?;
                s.contour_smoothness = parse(3)?;
                s.curve_complexity = parse(4)?;
            }
            for (key, values) in advanced_parameters(&s, stage)? {
                println!(
                    "{key} = {}",
                    values
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
        }
        _ => return Err(
            "Commands: inspect | derive palette|segment|smooth|fit [segmentation smoothness curve]"
                .into(),
        ),
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
