// Eulerian advection-diffusion-decay step for radar reflectivity.
//   dC/dt = -u . grad(C) + K lap(C) - lambda C
// Reflectivity is a transported scalar, not a conserved mass: the flux-form
// divergence is corrected by C*div(u) so convergent flow cannot pile echoes up
// into spurious intense cells. Fluxes are MC-limited MUSCL upwind.
// Velocities are in cells per sub-step; K is in cells^2 per sub-step.

struct Params {
    width: u32,
    height: u32,
    diffusion: f32,
    decay: f32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> c_in: array<f32>;
@group(0) @binding(2) var<storage, read_write> c_out: array<f32>;
@group(0) @binding(3) var<storage, read> vel_u: array<f32>;
@group(0) @binding(4) var<storage, read> vel_v: array<f32>;

// Ghost cells outside the domain hold zero (nothing flows in from outside).
fn cell(x: i32, y: i32) -> f32 {
    if (x < 0 || y < 0 || x >= i32(p.width) || y >= i32(p.height)) {
        return 0.0;
    }
    return c_in[u32(y) * p.width + u32(x)];
}

fn cell_neumann(x: i32, y: i32) -> f32 {
    let cx = clamp(x, 0, i32(p.width) - 1);
    let cy = clamp(y, 0, i32(p.height) - 1);
    return c_in[u32(cy) * p.width + u32(cx)];
}

fn vel(x: i32, y: i32) -> vec2<f32> {
    let cx = clamp(x, 0, i32(p.width) - 1);
    let cy = clamp(y, 0, i32(p.height) - 1);
    let i = u32(cy) * p.width + u32(cx);
    return vec2<f32>(vel_u[i], vel_v[i]);
}

fn minmod(a: f32, b: f32) -> f32 {
    if (a * b <= 0.0) {
        return 0.0;
    }
    return select(max(a, b), min(a, b), a > 0.0);
}

// Monotonised-central limiter: second-order accurate, much less smearing than minmod.
fn limited_slope(a: f32, b: f32) -> f32 {
    if (a * b <= 0.0) {
        return 0.0;
    }
    let m = min(min(2.0 * abs(a), 2.0 * abs(b)), 0.5 * abs(a + b));
    return select(-m, m, a > 0.0);
}

// Upwind-biased face value between cells (m1, m0 | p0, p1) for face velocity uf.
fn face(m1: f32, m0: f32, p0: f32, p1: f32, uf: f32) -> f32 {
    if (uf >= 0.0) {
        return m0 + 0.5 * limited_slope(m0 - m1, p0 - m0);
    }
    return p0 - 0.5 * limited_slope(p0 - m0, p1 - p0);
}

// Flux through the +x face of cell (x, y).
fn flux_x(x: i32, y: i32) -> f32 {
    let uf = 0.5 * (vel(x, y).x + vel(x + 1, y).x);
    let f = face(cell(x - 1, y), cell(x, y), cell(x + 1, y), cell(x + 2, y), uf);
    return uf * f;
}

// Flux through the +y (south) face of cell (x, y).
fn flux_y(x: i32, y: i32) -> f32 {
    let vf = 0.5 * (vel(x, y).y + vel(x, y + 1).y);
    let f = face(cell(x, y - 1), cell(x, y), cell(x, y + 1), cell(x, y + 2), vf);
    return vf * f;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.width || gid.y >= p.height) {
        return;
    }
    let x = i32(gid.x);
    let y = i32(gid.y);
    let c = cell(x, y);

    let ue = 0.5 * (vel(x, y).x + vel(x + 1, y).x);
    let uw = 0.5 * (vel(x - 1, y).x + vel(x, y).x);
    let vs = 0.5 * (vel(x, y).y + vel(x, y + 1).y);
    let vn = 0.5 * (vel(x, y - 1).y + vel(x, y).y);
    let div_u = (ue - uw) + (vs - vn);
    let adv = (flux_x(x, y) - flux_x(x - 1, y)) + (flux_y(x, y) - flux_y(x, y - 1)) - c * div_u;
    let lap = cell_neumann(x - 1, y) + cell_neumann(x + 1, y)
        + cell_neumann(x, y - 1) + cell_neumann(x, y + 1) - 4.0 * c;

    let next = (c - adv + p.diffusion * lap) * p.decay;
    c_out[gid.y * p.width + gid.x] = clamp(next, 0.0, 255.0);
}
