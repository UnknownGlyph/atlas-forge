// Shared map shader: one geometry, many projections.
// Vertices carry relative lon/lat in degrees (lon - lon0); projection happens here.

struct View {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,      // globe camera position (unit sphere at origin)
    center: vec4<f32>,       // 2D: cx, cy (projected units), zoom (px/unit), lon0 (deg)
    viewport: vec4<f32>,     // width px, height px, mode (0 = 2D, 1 = globe), projection id
    light: vec4<f32>,        // globe light direction
    line_color: array<vec4<f32>, 12>,
    line_width: array<vec4<f32>, 12>,
};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> colors: array<vec4<f32>>;

const PI: f32 = 3.14159265358979;
const DEG: f32 = 0.0174532925199433;
const MERC_MAX: f32 = 1.48442223;  // 85.0511 degrees

fn project2d(ll: vec2<f32>) -> vec2<f32> {
    let lon = ll.x * DEG;
    let lat = ll.y * DEG;
    let p = u32(view.viewport.w);
    if (p == 1u) {
        let l = clamp(lat, -MERC_MAX, MERC_MAX);
        return vec2<f32>(lon, log(tan(PI * 0.25 + l * 0.5)));
    }
    if (p == 2u) {
        return vec2<f32>(lon, 1.25 * log(tan(PI * 0.25 + 0.4 * lat)));
    }
    if (p == 3u) {
        // Equal Earth
        let a1 = 1.340264; let a2 = -0.081106; let a3 = 0.000893; let a4 = 0.003796;
        let t = asin(0.8660254 * sin(lat));
        let t2 = t * t;
        let t6 = t2 * t2 * t2;
        let x = 2.0 * 1.7320508 * lon * cos(t) / (3.0 * (9.0 * a4 * t6 * t2 + 7.0 * a3 * t6 + 3.0 * a2 * t2 + a1));
        let y = t * (a1 + a2 * t2 + t6 * (a3 + a4 * t2));
        return vec2<f32>(x, y);
    }
    return vec2<f32>(lon, lat);
}

fn sphere(ll: vec2<f32>) -> vec3<f32> {
    let lon = (ll.x + view.center.w) * DEG;
    let lat = ll.y * DEG;
    return vec3<f32>(cos(lat) * sin(lon), sin(lat), cos(lat) * cos(lon));
}

fn is_globe() -> bool {
    return view.viewport.z > 0.5;
}

fn to_clip(ll: vec2<f32>) -> vec4<f32> {
    if (is_globe()) {
        return view.view_proj * vec4<f32>(sphere(ll), 1.0);
    }
    let xy = (project2d(ll) - view.center.xy) * view.center.z / (view.viewport.xy * 0.5);
    return vec4<f32>(xy, 0.0, 1.0);
}

// Globe: is a surface point on the visible hemisphere?
fn visible(world: vec3<f32>) -> bool {
    return !is_globe() || dot(normalize(world), view.cam_pos.xyz) >= 1.0;
}

fn shade(world: vec3<f32>) -> f32 {
    if (!is_globe()) {
        return 1.0;
    }
    return 0.72 + 0.28 * max(dot(normalize(world), normalize(view.light.xyz)), 0.0);
}

struct FillOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world: vec3<f32>,
};

@vertex
fn vs_fill(@location(0) ll: vec2<f32>, @location(1) slot: u32) -> FillOut {
    var o: FillOut;
    o.pos = to_clip(ll);
    o.color = colors[slot];
    o.world = sphere(ll);
    return o;
}

@fragment
fn fs_fill(i: FillOut) -> @location(0) vec4<f32> {
    if (!visible(i.world)) {
        discard;
    }
    let s = shade(i.world);
    return vec4<f32>(i.color.rgb * s * i.color.a, i.color.a);
}

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world: vec3<f32>,
    @location(2) across: f32,
    @location(3) half_width: f32,
    // screen offset from the view's centre (px), and 1 for a coast band
    @location(4) screen: vec2<f32>,
    @location(5) band: f32,
};

@vertex
fn vs_line(@builtin(vertex_index) vi: u32, @location(0) p0: vec2<f32>, @location(1) p1: vec2<f32>, @location(2) cls: u32) -> LineOut {
    var ends = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    var sides = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let c0 = to_clip(p0);
    let c1 = to_clip(p1);
    let half = view.viewport.xy * 0.5;
    let s0 = c0.xy / c0.w * half;
    let s1 = c1.xy / c1.w * half;
    var dir = s1 - s0;
    let len = length(dir);
    if (len < 1e-6) {
        dir = vec2<f32>(1.0, 0.0);
    } else {
        dir = dir / len;
    }
    let nrm = vec2<f32>(-dir.y, dir.x);
    let hw = view.line_width[cls].x * 0.5;
    let w = hw + 1.0;  // anti-aliasing fringe
    let e = ends[vi];
    let sd = sides[vi];
    let s = mix(s0, s1, e) + dir * (e * 2.0 - 1.0) * hw + nrm * sd * w;
    let cw = mix(c0.w, c1.w, e);
    let cz = mix(c0.z / c0.w, c1.z / c1.w, e);
    var o: LineOut;
    o.pos = vec4<f32>(s / half * cw, cz * cw, cw);
    o.color = view.line_color[cls];
    o.world = mix(sphere(p0), sphere(p1), e);
    o.across = sd * w;
    o.half_width = hw;
    o.screen = s;
    o.band = select(0.0, 1.0, cls >= 9u);
    // a coast band closer to the flat map's left or right edge than its
    // own width would spill past the map: none there
    if (cls >= 9u && !is_globe()) {
        var near_edge = false;
        for (var k = 0; k < 2; k++) {
            let p = select(p1, p0, k == 0);
            let sp = select(s1, s0, k == 0);
            for (var side = -1.0; side < 2.0; side += 2.0) {
                let ce = to_clip(vec2<f32>(side * 180.0, p.y));
                let se = ce.xy / ce.w * half;
                if (abs(se.x - sp.x) < hw + 2.0) {
                    near_edge = true;
                }
            }
        }
        if (near_edge) {
            o.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        }
    }
    return o;
}

@fragment
fn fs_line(i: LineOut) -> @location(0) vec4<f32> {
    if (!visible(i.world)) {
        discard;
    }
    // coast bands stay on the globe
    if (i.band > 0.5 && is_globe() && length(i.screen) > view.light.w) {
        discard;
    }
    let a = clamp(i.half_width + 0.5 - abs(i.across), 0.0, 1.0) * i.color.a;
    return vec4<f32>(i.color.rgb * a, a);
}
