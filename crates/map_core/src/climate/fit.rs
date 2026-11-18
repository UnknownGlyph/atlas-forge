use super::*;

/// Bounded coordinate search, evaluated on the same geography and monthly
/// model used by the final map. A trial pins its input while resolving links.
pub fn fit(
    settings: &Settings,
    ground: &[Ground],
    region: Region,
    year: Option<i32>,
) -> Result<(Settings, [f64; 5]), String> {
    let targets = &settings.goals;
    let goals = [
        targets.temperature,
        targets.rain,
        targets.ice,
        targets.dry,
        targets.season,
    ];
    if goals.iter().all(Option::is_none) {
        return Err("先启用至少一个气候目标。".into());
    }
    let score = |v: [f64; 5]| -> f64 {
        goals
            .iter()
            .enumerate()
            .filter_map(|(k, g)| {
                g.map(|goal| ((v[k] - goal) / [3., 150., 0.08, 0.08, 5.][k]).powi(2))
            })
            .sum()
    };
    let evaluate = |s: &Settings| -> Result<[f64; 5], String> {
        Ok(Scenario::new(s.clone(), year, region, ground)?.summary(region))
    };
    let mut best = settings.clone();
    best.planet = best.planet.resolve()?;
    let mut metrics = evaluate(&best)?;
    let mut error = score(metrics);
    use Param::*;
    // Climate controls first. Redundant physical dimensions are still
    // available when an author has locked the usual controls.
    let knobs = [
        Greenhouse, Vapour, Transport, Tilt, Albedo, Ocean, Flux, Pressure, Spin, Ecc, CO2,
        Methane, AirDensity, Distance, Lum, Period, StarTemp, StarMass, StarRadius, Gravity, Mass,
        Radius, Density, Substellar,
    ];
    if !knobs
        .iter()
        .any(|&k| best.planet.controls[k as usize] == Control::Auto)
    {
        return Err("目标搜索需要至少一个自动参数。".into());
    }
    for step in [0.30, 0.15, 0.075, 0.035] {
        for p in knobs {
            if best.planet.controls[p as usize] != Control::Auto || error < 0.005 {
                continue;
            }
            let base = best.planet.clone();
            let (_, reference, lo, hi, _) = p.spec();
            for dir in [-1., 1.] {
                let mut trial = base.clone();
                let delta = match p {
                    Tilt => 45.,
                    Substellar => 90.,
                    Ecc => 0.4,
                    Albedo | Ocean => 0.5,
                    _ => base[p].abs().max(reference.abs() * 0.2).max(0.1),
                };
                trial[p] = (base[p] + dir * step * delta).clamp(lo, hi);
                if trial[p] == base[p] {
                    continue;
                }
                trial.controls[p as usize] = Control::Fixed;
                let Ok(mut planet) = trial.resolve() else {
                    continue;
                };
                planet.controls[p as usize] = Control::Auto;
                let candidate = Settings {
                    planet,
                    goals: targets.clone(),
                };
                let v = evaluate(&candidate)?;
                let next = score(v);
                if next + 1e-8 < error {
                    best = candidate;
                    metrics = v;
                    error = next;
                }
            }
        }
    }
    Ok((best, metrics))
}
