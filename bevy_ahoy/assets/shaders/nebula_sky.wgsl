#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::globals
#import bevy_pbr::pbr_fragment::pbr_input_from_vertex_output

struct NebulaSkySettings {
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    nebula_a: vec4<f32>,
    nebula_b: vec4<f32>,
    star: vec4<f32>,
    halo: vec4<f32>,
    params_a: vec4<f32>,
    params_b: vec4<f32>,
    params_c: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: NebulaSkySettings;

fn saturate(value: f32) -> f32 {
    return clamp(value, 0.0, 1.0);
}

fn safe_normalize(vector: vec3<f32>) -> vec3<f32> {
    return vector / sqrt(max(dot(vector, vector), 1e-6));
}

fn hash13(point: vec3<f32>) -> f32 {
    var q = fract(point * 0.1031);
    q += dot(q, q.yzx + vec3<f32>(33.33));
    return fract((q.x + q.y) * q.z);
}

fn noise(point: vec3<f32>) -> f32 {
    let cell = floor(point);
    let local = fract(point);
    let smooth_local = local * local * (3.0 - 2.0 * local);

    let a = hash13(cell);
    let b = hash13(cell + vec3<f32>(1.0, 0.0, 0.0));
    let c = hash13(cell + vec3<f32>(0.0, 1.0, 0.0));
    let d = hash13(cell + vec3<f32>(1.0, 1.0, 0.0));
    let e = hash13(cell + vec3<f32>(0.0, 0.0, 1.0));
    let f = hash13(cell + vec3<f32>(1.0, 0.0, 1.0));
    let g = hash13(cell + vec3<f32>(0.0, 1.0, 1.0));
    let h = hash13(cell + vec3<f32>(1.0, 1.0, 1.0));

    let x00 = mix(a, b, smooth_local.x);
    let x10 = mix(c, d, smooth_local.x);
    let x01 = mix(e, f, smooth_local.x);
    let x11 = mix(g, h, smooth_local.x);
    let y0 = mix(x00, x10, smooth_local.y);
    let y1 = mix(x01, x11, smooth_local.y);

    return mix(y0, y1, smooth_local.z);
}

fn fbm(point: vec3<f32>) -> f32 {
    var value = 0.0;
    var amplitude = 0.5;
    var position = point;

    for (var octave = 0; octave < 5; octave += 1) {
        value += noise(position) * amplitude;
        position = position * 2.03 + vec3<f32>(17.0, 29.0, 13.0);
        amplitude *= 0.5;
    }

    return value;
}

@fragment
fn fragment(
    mesh: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> @location(0) vec4<f32> {
    let pbr_input = pbr_input_from_vertex_output(mesh, is_front, false);
    let direction = safe_normalize(vec3<f32>(-pbr_input.V.x, pbr_input.V.y, -pbr_input.V.z));

    let drift = globals.time * material.params_b.xy;
    let primary_sample = direction * material.params_a.x + vec3<f32>(drift.x, drift.y, 0.0);
    let secondary_sample =
        direction.yzx * material.params_a.y + vec3<f32>(-drift.y * 1.5, drift.x * 1.2, 4.7);
    let wisp_sample =
        direction.zxy * (material.params_a.y * 2.4) + vec3<f32>(1.4, -2.1, drift.x - drift.y);
    let cloud_large = fbm(primary_sample);
    let cloud_detail = fbm(secondary_sample);
    let wisps = fbm(wisp_sample);

    let horizon = smoothstep(-0.18, 0.32, direction.y);
    let clean_zenith = pow(saturate(direction.y * 0.5 + 0.5), 3.1);
    let horizon_band = 1.0 - smoothstep(0.18, 0.74, abs(direction.y - 0.02));

    let halo_dir = safe_normalize(material.params_c.xyz);
    let secondary_mass_dir =
        safe_normalize(vec3<f32>(-halo_dir.z * 0.78 - 0.22, 0.08, halo_dir.x * 0.78));
    let mass_a_dir = smoothstep(0.26, 0.82, dot(direction, halo_dir) * 0.5 + 0.5);
    let mass_b_dir = smoothstep(0.24, 0.8, dot(direction, secondary_mass_dir) * 0.5 + 0.5);

    let mass_a = smoothstep(0.46, 0.82, cloud_large * 0.72 + cloud_detail * 0.42)
        * horizon_band
        * mass_a_dir
        * (1.0 - clean_zenith);
    let mass_b = smoothstep(0.48, 0.82, cloud_detail * 0.78 + wisps * 0.32)
        * horizon_band
        * mass_b_dir
        * (1.0 - clean_zenith * 0.9);
    let wisp_mask =
        smoothstep(0.56, 0.88, wisps) * horizon_band * (0.3 + 0.7 * max(mass_a, mass_b));

    var color = mix(material.zenith.rgb, material.horizon.rgb, horizon);
    color = mix(color, material.zenith.rgb, clean_zenith * 0.24);
    color += material.nebula_a.rgb * mass_a * material.params_b.z;
    color += material.nebula_b.rgb * (mass_b * material.params_b.w + wisp_mask * 0.15);

    let halo = pow(saturate(dot(direction, halo_dir)), material.params_c.w)
        * smoothstep(-0.18, 0.22, direction.y)
        * (0.5 + 0.5 * horizon_band);
    color += material.halo.rgb * material.halo.a * halo;

    let star_grid = direction * material.params_a.z;
    let star_cluster_grid = direction * material.params_a.w;
    let cluster_mask = smoothstep(
        0.82,
        0.985,
        hash13(floor(star_cluster_grid) + vec3<f32>(19.0, 7.0, 13.0)),
    );
    let star_cell = floor(star_grid);
    let star_local = fract(star_grid) - 0.5;
    let star_seed = hash13(star_cell + vec3<f32>(3.1, 5.7, 9.2));
    let star_size = mix(0.055, 0.016, star_seed);
    let star_core = smoothstep(star_size, 0.0, length(star_local));
    let star_presence = step(0.9985 - cluster_mask * 0.0014, star_seed);
    let twinkle = 0.72 + 0.28 * sin(globals.time * (1.0 + star_seed * 5.0) + star_seed * 6.28318530718);
    let stars = star_core
        * star_presence
        * twinkle
        * (0.4 + 0.6 * cluster_mask)
        * (1.0 - max(mass_a, mass_b) * 0.55)
        * (1.0 - horizon_band * 0.35);
    color += material.star.rgb * stars * material.star.a;

    return vec4<f32>(color, 1.0);
}
