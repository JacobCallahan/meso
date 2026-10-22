// GPU kernels for pyramidal Lucas-Kanade optical flow.
// All kernels share one parameter block and a fixed set of storage bindings;
// each entry point only touches the bindings it needs (auto pipeline layout).

struct Params {
    w: u32,
    h: u32,
    w2: u32,
    h2: u32,
    radius: u32,
    pad0: u32,
    lambda: f32,
    scale: f32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> s0: array<f32>;
@group(0) @binding(2) var<storage, read> s1: array<f32>;
@group(0) @binding(3) var<storage, read> su: array<f32>;
@group(0) @binding(4) var<storage, read> sv: array<f32>;
@group(0) @binding(5) var<storage, read_write> d0: array<f32>;
@group(0) @binding(6) var<storage, read_write> d1: array<f32>;
@group(0) @binding(7) var<storage, read_write> d2: array<f32>;

const GROUP: u32 = 16u;

// Bilinear sample of a w x h image stored in `img` (index via closure-free helpers below).
// Matches the CPU reference: nearest inside the half-cell border, zero beyond it.
fn bilerp_s0(c: f32, r: f32, w: u32, h: u32) -> f32 {
    let wf = f32(w);
    let hf = f32(h);
    if (c < 0.0 || r < 0.0 || c >= wf - 1.0 || r >= hf - 1.0) {
        if (c >= -0.5 && c <= wf - 0.5 && r >= -0.5 && r <= hf - 0.5) {
            let ic = u32(clamp(c, 0.0, wf - 1.0));
            let ir = u32(clamp(r, 0.0, hf - 1.0));
            return s0[ir * w + ic];
        }
        return 0.0;
    }
    let c0 = u32(floor(c));
    let r0 = u32(floor(r));
    let fc = c - f32(c0);
    let fr = r - f32(r0);
    let top = s0[r0 * w + c0] * (1.0 - fc) + s0[r0 * w + c0 + 1u] * fc;
    let bot = s0[(r0 + 1u) * w + c0] * (1.0 - fc) + s0[(r0 + 1u) * w + c0 + 1u] * fc;
    return top * (1.0 - fr) + bot * fr;
}

fn bilerp_s1(c: f32, r: f32, w: u32, h: u32) -> f32 {
    let wf = f32(w);
    let hf = f32(h);
    if (c < 0.0 || r < 0.0 || c >= wf - 1.0 || r >= hf - 1.0) {
        if (c >= -0.5 && c <= wf - 0.5 && r >= -0.5 && r <= hf - 0.5) {
            let ic = u32(clamp(c, 0.0, wf - 1.0));
            let ir = u32(clamp(r, 0.0, hf - 1.0));
            return s1[ir * w + ic];
        }
        return 0.0;
    }
    let c0 = u32(floor(c));
    let r0 = u32(floor(r));
    let fc = c - f32(c0);
    let fr = r - f32(r0);
    let top = s1[r0 * w + c0] * (1.0 - fc) + s1[r0 * w + c0 + 1u] * fc;
    let bot = s1[(r0 + 1u) * w + c0] * (1.0 - fc) + s1[(r0 + 1u) * w + c0 + 1u] * fc;
    return top * (1.0 - fr) + bot * fr;
}

// Velocity sample (su, sv) of dims (w, h); clamps outside like MotionField::sample_uv.
fn sample_uv(c: f32, r: f32, w: u32, h: u32) -> vec2<f32> {
    let wf = f32(w);
    let hf = f32(h);
    if (c < 0.0 || r < 0.0 || c >= wf - 1.0 || r >= hf - 1.0) {
        let ic = u32(clamp(c, 0.0, wf - 1.0));
        let ir = u32(clamp(r, 0.0, hf - 1.0));
        let i = ir * w + ic;
        return vec2<f32>(su[i], sv[i]);
    }
    let c0 = u32(floor(c));
    let r0 = u32(floor(r));
    let fc = c - f32(c0);
    let fr = r - f32(r0);
    let i00 = r0 * w + c0;
    let i01 = i00 + 1u;
    let i10 = i00 + w;
    let i11 = i10 + 1u;
    let ut = su[i00] * (1.0 - fc) + su[i01] * fc;
    let ub = su[i10] * (1.0 - fc) + su[i11] * fc;
    let vt = sv[i00] * (1.0 - fc) + sv[i01] * fc;
    let vb = sv[i10] * (1.0 - fc) + sv[i11] * fc;
    return vec2<f32>(ut * (1.0 - fr) + ub * fr, vt * (1.0 - fr) + vb * fr);
}

// 2x2 mean downsample: s0 (w,h) -> d0 (w2,h2).
@compute @workgroup_size(16, 16)
fn downsample(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.w2 || g.y >= p.h2) { return; }
    let r0 = min(g.y * 2u, p.h - 1u);
    let r1 = min(r0 + 1u, p.h - 1u);
    let c0 = min(g.x * 2u, p.w - 1u);
    let c1 = min(c0 + 1u, p.w - 1u);
    d0[g.y * p.w2 + g.x] = 0.25 * (s0[r0 * p.w + c0] + s0[r0 * p.w + c1] + s0[r1 * p.w + c0] + s0[r1 * p.w + c1]);
}

// Upsample flow (su,sv) (w,h) -> (d0,d1) (w2,h2), multiplied by p.scale.
@compute @workgroup_size(16, 16)
fn upsample_flow(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.w2 || g.y >= p.h2) { return; }
    let sx = f32(p.w) / f32(p.w2);
    let sy = f32(p.h) / f32(p.h2);
    let src_c = (f32(g.x) + 0.5) * sx - 0.5;
    let src_r = (f32(g.y) + 0.5) * sy - 0.5;
    let uv = sample_uv(src_c, src_r, p.w, p.h) * p.scale;
    d0[g.y * p.w2 + g.x] = uv.x;
    d1[g.y * p.w2 + g.x] = uv.y;
}

// Warp curr (s1) by flow (su,sv), build structure-tensor terms against prev (s0).
// Output d0: 5 floats per pixel [ixx, iyy, ixy, ixt, iyt].
@compute @workgroup_size(16, 16)
fn lk_terms(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.w || g.y >= p.h) { return; }
    let w = p.w;
    let h = p.h;
    let c = g.x;
    let r = g.y;
    let cp = select(c - 1u, 0u, c == 0u);
    let cn = min(c + 1u, w - 1u);
    let rp = select(r - 1u, 0u, r == 0u);
    let rn = min(r + 1u, h - 1u);

    let idx = r * w + c;
    let fu = su[idx];
    let fv = sv[idx];
    let cur_c = f32(c) + fu;
    let cur_r = f32(r) + fv;

    // Warped current values at the center and 4 neighbors.
    let wl = bilerp_s1(f32(cp) + su[r * w + cp], f32(r) + sv[r * w + cp], w, h);
    let wr = bilerp_s1(f32(cn) + su[r * w + cn], f32(r) + sv[r * w + cn], w, h);
    let wt = bilerp_s1(f32(c) + su[rp * w + c], f32(rp) + sv[rp * w + c], w, h);
    let wb = bilerp_s1(f32(c) + su[rn * w + c], f32(rn) + sv[rn * w + c], w, h);
    let wc = bilerp_s1(cur_c, cur_r, w, h);

    let avg_left = 0.5 * (s0[r * w + cp] + wl);
    let avg_right = 0.5 * (s0[r * w + cn] + wr);
    let avg_top = 0.5 * (s0[rp * w + c] + wt);
    let avg_bottom = 0.5 * (s0[rn * w + c] + wb);

    let ix = 0.5 * (avg_right - avg_left);
    let iy = 0.5 * (avg_bottom - avg_top);
    let it = wc - s0[idx];

    let o = idx * 5u;
    d0[o] = ix * ix;
    d0[o + 1u] = iy * iy;
    d0[o + 2u] = ix * iy;
    d0[o + 3u] = ix * it;
    d0[o + 4u] = iy * it;
}

// Separable edge-normalised box mean over 5-channel buffers: s0 -> d0.
@compute @workgroup_size(16, 16)
fn blur_h(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.w || g.y >= p.h) { return; }
    let lo = max(i32(g.x) - i32(p.radius), 0);
    let hi = min(i32(g.x) + i32(p.radius), i32(p.w) - 1);
    let inv = 1.0 / f32(hi - lo + 1);
    for (var k = 0u; k < 5u; k++) {
        var sum = 0.0;
        for (var x = lo; x <= hi; x++) {
            sum += s0[(g.y * p.w + u32(x)) * 5u + k];
        }
        d0[(g.y * p.w + g.x) * 5u + k] = sum * inv;
    }
}

@compute @workgroup_size(16, 16)
fn blur_v(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.w || g.y >= p.h) { return; }
    let lo = max(i32(g.y) - i32(p.radius), 0);
    let hi = min(i32(g.y) + i32(p.radius), i32(p.h) - 1);
    let inv = 1.0 / f32(hi - lo + 1);
    for (var k = 0u; k < 5u; k++) {
        var sum = 0.0;
        for (var y = lo; y <= hi; y++) {
            sum += s0[(u32(y) * p.w + g.x) * 5u + k];
        }
        d0[(g.y * p.w + g.x) * 5u + k] = sum * inv;
    }
}

// Regularised 2x2 LK solve; blurred terms in s0, flow updated in place (d0=u, d1=v).
@compute @workgroup_size(16, 16)
fn lk_solve(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.w || g.y >= p.h) { return; }
    let idx = g.y * p.w + g.x;
    let o = idx * 5u;
    let a00 = s0[o] + p.lambda;
    let a11 = s0[o + 1u] + p.lambda;
    let a01 = s0[o + 2u];
    let b0 = -s0[o + 3u];
    let b1 = -s0[o + 4u];
    let det = a00 * a11 - a01 * a01;
    if (det > 1e-6) {
        let du = (a11 * b0 - a01 * b1) / det;
        let dv = (a00 * b1 - a01 * b0) / det;
        d0[idx] = d0[idx] + clamp(du, -2.0, 2.0);
        d1[idx] = d1[idx] + clamp(dv, -2.0, 2.0);
    }
}

// Echo mask from the image in s0 -> d0.
@compute @workgroup_size(16, 16)
fn diffuse_init(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.w || g.y >= p.h) { return; }
    let idx = g.y * p.w + g.x;
    d0[idx] = select(0.0, 1.0, s0[idx] > 5.0);
}

// One normalised-convolution step (5x5 window): (su, sv, s0=mask) -> (d0, d1, d2).
@compute @workgroup_size(16, 16)
fn diffuse_step(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.w || g.y >= p.h) { return; }
    let idx = g.y * p.w + g.x;
    let m = s0[idx];
    var ou = su[idx];
    var ov = sv[idx];
    var om = m;
    if (m < 0.1) {
        let xlo = max(i32(g.x) - 2, 0);
        let xhi = min(i32(g.x) + 2, i32(p.w) - 1);
        let ylo = max(i32(g.y) - 2, 0);
        let yhi = min(i32(g.y) + 2, i32(p.h) - 1);
        var sm = 0.0;
        var sU = 0.0;
        var sV = 0.0;
        for (var y = ylo; y <= yhi; y++) {
            for (var x = xlo; x <= xhi; x++) {
                let j = u32(y) * p.w + u32(x);
                let mj = s0[j];
                sm += mj;
                sU += su[j] * mj;
                sV += sv[j] * mj;
            }
        }
        let n = f32((xhi - xlo + 1) * (yhi - ylo + 1));
        let bm = sm / n;
        if (bm > 1e-4) {
            ou = (sU / n) / bm;
            ov = (sV / n) / bm;
            om = min(bm * 1.5, 1.0);
        }
    }
    d0[idx] = ou;
    d1[idx] = ov;
    d2[idx] = om;
}

// Apply diffused velocity to clear-air cells: s0=image, s1=final mask, (su,sv)=diffused, (d0,d1)=field.
@compute @workgroup_size(16, 16)
fn diffuse_apply(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= p.w || g.y >= p.h) { return; }
    let idx = g.y * p.w + g.x;
    if (s0[idx] <= 5.0 && s1[idx] > 0.01) {
        d0[idx] = su[idx];
        d1[idx] = sv[idx];
    }
}
