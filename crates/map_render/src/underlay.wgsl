// Reference image under the map. A full-screen triangle; every fragment is
// unprojected to lon/lat (2D inverse projection or globe ray cast), then
// projected into the image's own projection and through its affine
// georeference to texture coordinates. Exact in every view, no seam meshes.

struct U {
    inv_view_proj: mat4x4<f32>,
    center: vec4<f32>,    // 2D: cx, cy (projected units), zoom (px/unit), view lon0 (deg)
    viewport: vec4<f32>,  // width px, height px, mode (0 = 2D, 1 = globe), view projection id
    light: vec4<f32>,     // globe light direction
    m0: vec4<f32>,        // u = m0.x * x + m0.y * y + m0.z   (normalised 0..1)
    m1: vec4<f32>,        // v = m1.x * x + m1.y * y + m1.z
    src: vec4<f32>,       // image projection id, image lon0 (deg), opacity, -
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

const PI: f32 = 3.14159265358979;
const DEG: f32 = 0.0174532925199433;
const MERC_MAX: f32 = 1.48442223;
const A1: f32 = 1.340264;
const A2: f32 = -0.081106;
const A3: f32 = 0.000893;
const A4: f32 = 0.003796;
const M: f32 = 0.8660254;

// radians in, sphere units out (same formulas as map.wgsl / projection.rs)
fn forward(p: u32, lon: f32, lat: f32) -> vec2<f32> {
    if (p == 1u) {
        let l = clamp(lat, -MERC_MAX, MERC_MAX);
        return vec2<f32>(lon, log(tan(PI * 0.25 + l * 0.5)));
    }
    if (p == 2u) {
        return vec2<f32>(lon, 1.25 * log(tan(PI * 0.25 + 0.4 * lat)));
    }
    if (p == 3u) {
        let t = asin(M * sin(lat));
        let t2 = t * t;
        let t6 = t2 * t2 * t2;
        let x = 2.0 * 1.7320508 * lon * cos(t) / (3.0 * (9.0 * A4 * t6 * t2 + 7.0 * A3 * t6 + 3.0 * A2 * t2 + A1));
        let y = t * (A1 + A2 * t2 + t6 * (A3 + A4 * t2));
        return vec2<f32>(x, y);
    }
    return vec2<f32>(lon, lat);
}

// sphere units in, radians out; z = 1 when the point is on the map
fn inverse(p: u32, xy: vec2<f32>) -> vec3<f32> {
    var lon = xy.x;
    var lat = xy.y;
    if (p == 1u) {
        lat = 2.0 * atan(exp(xy.y)) - PI * 0.5;
    } else if (p == 2u) {
        lat = (atan(exp(xy.y / 1.25)) - PI * 0.25) / 0.4;
    } else if (p == 3u) {
        var t = xy.y;
        for (var i = 0; i < 8; i++) {
            let t2 = t * t;
            let t6 = t2 * t2 * t2;
            let fy = t * (A1 + A2 * t2 + t6 * (A3 + A4 * t2)) - xy.y;
            let fpy = A1 + 3.0 * A2 * t2 + t6 * (7.0 * A3 + 9.0 * A4 * t2);
            t = t - fy / fpy;
        }
        let t2 = t * t;
        let t6 = t2 * t2 * t2;
        lon = 3.0 * xy.x * (A1 + 3.0 * A2 * t2 + t6 * (7.0 * A3 + 9.0 * A4 * t2)) / (2.0 * 1.7320508 * cos(t));
        let s = sin(t) / M;
        if (abs(s) > 1.0) {
            return vec3<f32>(0.0, 0.0, 0.0);
        }
        lat = asin(s);
    }
    let ok = abs(lon) <= PI + 1e-5 && abs(lat) <= PI * 0.5 + 1e-5;
    return vec3<f32>(lon, lat, select(0.0, 1.0, ok));
}

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> Out {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: Out;
    o.pos = vec4<f32>(p[vi], 0.0, 1.0);
    o.ndc = p[vi];
    return o;
}

@fragment
fn fs(i: Out) -> @location(0) vec4<f32> {
    var ok = true;
    var lon = 0.0;   // absolute, degrees
    var lat = 0.0;
    var shade = 1.0;
    if (u.viewport.z > 0.5) {
        let n = u.inv_view_proj * vec4<f32>(i.ndc, 0.0, 1.0);
        let f = u.inv_view_proj * vec4<f32>(i.ndc, 1.0, 1.0);
        let o = n.xyz / n.w;
        let d = normalize(f.xyz / f.w - o);
        let b = dot(o, d);
        let c = dot(o, o) - 1.0;
        let disc = b * b - c;
        ok = disc >= 0.0;
        let hit = o + d * (-b - sqrt(max(disc, 0.0)));
        lon = atan2(hit.x, hit.z) / DEG;
        lat = asin(clamp(hit.y, -1.0, 1.0)) / DEG;
        shade = 0.72 + 0.28 * max(dot(normalize(hit), normalize(u.light.xyz)), 0.0);
    } else {
        let xy = u.center.xy + i.ndc * u.viewport.xy * 0.5 / u.center.z;
        let ll = inverse(u32(u.viewport.w), xy);
        ok = ll.z > 0.5;
        lon = ll.x / DEG + u.center.w;
        lat = ll.y / DEG;
    }
    var rel = lon - u.src.y;
    rel = rel - 360.0 * floor((rel + 180.0) / 360.0);
    let s = forward(u32(u.src.x), rel * DEG, lat * DEG);
    let uv = vec2<f32>(dot(u.m0.xyz, vec3<f32>(s, 1.0)), dot(u.m1.xyz, vec3<f32>(s, 1.0)));
    // mip selection: where the image wraps around the planet, uv jumps by
    // a whole image width; the derivative of a shifted copy is smooth there
    let w = fract(uv + 0.5);
    var dx = dpdx(uv);
    var dy = dpdy(uv);
    let dxw = dpdx(w);
    let dyw = dpdy(w);
    dx = select(dx, dxw, abs(dxw) < abs(dx));
    dy = select(dy, dyw, abs(dyw) < abs(dy));
    let c = textureSampleGrad(tex, smp, uv, dx, dy);
    if (!ok || uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) {
        return vec4<f32>(0.0);
    }
    if (u.src.w > 0.5) {
        return vec4<f32>(c.rgb * select(shade, 1.0, u.src.w > 1.5), c.a);
    }
    let a = c.a * u.src.z;
    return vec4<f32>(c.rgb * shade * a, a);
}
