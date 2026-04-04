#import bevy_pbr::{
    forward_io::{FragmentOutput, VertexOutput},
    mesh_view_bindings::{globals, view},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct WorldSurfaceSettings {
    accent: vec4<f32>,
    secondary: vec4<f32>,
    emissive: vec4<f32>,
    atmosphere: vec4<f32>,
    params_a: vec4<f32>,
    params_b: vec4<f32>,
    params_c: vec4<f32>,
    params_d: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> material: WorldSurfaceSettings;

fn saturate(value: f32) -> f32 {
    return clamp(value, 0.0, 1.0);
}

fn safe_normalize(vector: vec3<f32>) -> vec3<f32> {
    let length_squared = max(dot(vector, vector), 1e-6);
    return vector / sqrt(length_squared);
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn line(distance_to_line: f32, width: f32, feather: f32) -> f32 {
    return 1.0 - smoothstep(width, width + feather, distance_to_line);
}

fn tile_edge_distance(tile_uv: vec2<f32>) -> f32 {
    return min(min(tile_uv.x, tile_uv.y), min(1.0 - tile_uv.x, 1.0 - tile_uv.y));
}

fn hash13(point: vec3<f32>) -> f32 {
    var q = fract(point * 0.1031);
    q += dot(q, q.yzx + vec3<f32>(33.33));
    return fract((q.x + q.y) * q.z);
}

fn sky_reflection_color(normal: vec3<f32>, view_dir: vec3<f32>) -> vec3<f32> {
    let reflected = reflect(-view_dir, normal);
    let reflected_up = pow(saturate(reflected.y * 0.5 + 0.5), 1.8);
    return mix(material.atmosphere.rgb, material.accent.rgb, reflected_up);
}

@fragment
fn fragment(
    in_: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var in = in_;
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let surf_mask = step(0.5, material.params_a.x);
    let platform_mask = 1.0 - surf_mask;
    let world_pos = in.world_position.xyz;
    let view_dir = pbr_input.V;
    let normal = pbr_input.N;
    let top_face = saturate(normal.y * 0.5 + 0.5);
    let side_face = 1.0 - top_face;
    let underside = saturate(-normal.y);
    let view_dot = saturate(dot(normal, view_dir));
    let fresnel = pow(1.0 - view_dot, 3.1);
    let motion = material.params_d.x;
    let speed_boost = 1.0 + motion * 0.22;
    let camera_distance = length(world_pos - view.world_position.xyz);
    let distance_t = smoothstep(material.params_c.x, material.params_c.y, camera_distance);
    let sky_reflection = sky_reflection_color(normal, view_dir);

    var base_rgb = pbr_input.material.base_color.rgb;
    var emissive_rgb = pbr_input.material.emissive.rgb;

    let stripe_mask = smoothstep(0.58, 0.92, luminance(base_rgb));

#ifdef VERTEX_UVS_A
    let uv = in.uv;
#else
    let uv = world_pos.xz * 0.12;
#endif

    let tile_uv = fract(uv);
    let edge_distance = tile_edge_distance(tile_uv);
    let edge_frame = 1.0 - smoothstep(material.params_a.y, material.params_a.y + 0.04, edge_distance);
    let silhouette = edge_frame * 0.45 + fresnel * 0.55;

    let dpdx_pos = dpdx(world_pos);
    let dpdy_pos = dpdy(world_pos);
    let duv_dx = dpdx(uv);
    let duv_dy = dpdy(uv);
    let surf_tangent = safe_normalize(dpdx_pos * duv_dy.y - dpdy_pos * duv_dx.y);
    let tangent_align = pow(saturate(1.0 - abs(dot(surf_tangent, view_dir))), 2.2);

    let flow_u = uv.x * material.params_b.z - globals.time * material.params_b.x * speed_boost;
    let flow_wave = 0.5 + 0.5 * sin(flow_u + uv.y * material.params_b.w + world_pos.y * 0.08);
    let flow_scan = 1.0
        - smoothstep(
            0.12,
            0.24,
            abs(
                fract(
                    uv.x * material.params_d.y
                        - globals.time * 0.1 * speed_boost
                        + uv.y * 0.08,
                ) - 0.5,
            ),
        );
    let lane_threads = 1.0 - smoothstep(0.1, 0.24, abs(fract(uv.y * material.params_b.y) - 0.5));
    let surf_energy = stripe_mask
        * lane_threads
        * mix(0.35, 1.0, flow_wave)
        * (0.55 + 0.45 * flow_scan);
    let surf_glint = tangent_align * (0.35 + 0.65 * flow_scan) * (0.4 + 0.6 * fresnel);
    let surf_body = (
        mix(
            base_rgb * 0.28 + material.atmosphere.rgb * 0.04,
            base_rgb * 0.62 + material.secondary.rgb * 0.08,
            top_face,
        )
            * (1.0 - underside * material.params_a.w)
    ) + material.accent.rgb * silhouette * 0.04
        + sky_reflection
            * material.params_c.z
            * (0.08 + 0.18 * fresnel)
            * (0.25 + 0.75 * top_face);

    base_rgb = mix(base_rgb, surf_body, surf_mask);
    emissive_rgb +=
        material.emissive.rgb * surf_energy * (0.32 + 0.68 * top_face) * speed_boost * surf_mask;
    emissive_rgb += material.secondary.rgb * flow_scan * stripe_mask * 0.1 * surf_mask;
    emissive_rgb += material.accent.rgb * surf_glint * 0.14 * speed_boost * surf_mask;

    pbr_input.material.perceptual_roughness = mix(
        pbr_input.material.perceptual_roughness,
        saturate(mix(0.36, 0.14, stripe_mask) - surf_glint * 0.06),
        surf_mask,
    );
    pbr_input.material.clearcoat = mix(pbr_input.material.clearcoat, 0.94, surf_mask);
    pbr_input.material.clearcoat_perceptual_roughness = mix(
        pbr_input.material.clearcoat_perceptual_roughness,
        0.05,
        surf_mask,
    );

    let panel_edge = tile_edge_distance(tile_uv);
    let panel_feather = max(fwidth(panel_edge) * 2.0, 0.003);
    let inset_mask = smoothstep(
        material.params_a.z - panel_feather,
        material.params_a.z + panel_feather,
        panel_edge,
    );
    let inset_frame = (
        1.0
            - smoothstep(
                material.params_a.y - panel_feather,
                material.params_a.y + panel_feather,
                panel_edge,
            )
    ) * inset_mask;
    let bevel_band = smoothstep(
        material.params_a.z + 0.045 - panel_feather,
        material.params_a.z + 0.045 + panel_feather,
        panel_edge,
    ) * (
        1.0
            - smoothstep(
                material.params_a.z + 0.13 - panel_feather,
                material.params_a.z + 0.13 + panel_feather,
                panel_edge,
            )
    );
    let panel_depth = smoothstep(
        material.params_a.z + 0.03,
        material.params_a.z + 0.22,
        panel_edge,
    );
    let line_feather = max(fwidth(tile_uv.x) * 2.0, 0.003);
    let center_line = 1.0
        - smoothstep(
            0.016 - line_feather,
            0.016 + line_feather,
            abs(tile_uv.x - 0.5),
        );
    let detail_uv = world_pos.xz * material.params_b.z;
    let detail_phase = detail_uv.x * 0.92 + detail_uv.y * 0.16;
    let detail_feather = max(fwidth(detail_phase) * 2.0, 0.01);
    let micro_lines = 1.0
        - smoothstep(
            0.44 - detail_feather,
            0.44 + detail_feather,
            abs(fract(detail_phase) - 0.5),
        );
    let scan_phase = detail_uv.y * 0.24 - globals.time * material.params_b.x * 0.16 * speed_boost;
    let scan_band = 1.0
        - smoothstep(
            0.34,
            0.46,
            abs(fract(scan_phase) - 0.5),
        );
    let energy_scan = 1.0
        - smoothstep(
            0.22,
            0.38,
            abs(fract(tile_uv.y * material.params_b.w - globals.time * material.params_b.x * 0.14 * speed_boost) - 0.5),
        );
    let border_glow = inset_frame * (0.78 + 0.22 * energy_scan);
    let body_rgb = mix(
        vec3<f32>(0.008, 0.006, 0.014),
        base_rgb * 0.18 + material.atmosphere.rgb * 0.04,
        top_face * 0.66 + 0.1,
    );
    let panel_tint = mix(
        material.atmosphere.rgb * 0.45,
        material.secondary.rgb * 0.1 + material.accent.rgb * 0.018,
        0.3 + 0.7 * top_face,
    );
    let panel_rgb = panel_tint
        + material.secondary.rgb * panel_depth * 0.06
        + material.accent.rgb * bevel_band * 0.022
        + material.accent.rgb * center_line * 0.016
        + material.secondary.rgb * micro_lines * inset_mask * top_face * 0.018
        + material.emissive.rgb * scan_band * inset_mask * top_face * 0.05;

    var platform_rgb = body_rgb;
    platform_rgb = mix(platform_rgb, panel_rgb, inset_mask * top_face * 0.82);
    platform_rgb += material.accent.rgb * border_glow * (0.026 + 0.035 * top_face);
    platform_rgb += sky_reflection
        * material.params_c.z
        * (0.016 + 0.04 * fresnel)
        * (0.3 + 0.7 * top_face);
    platform_rgb *= 1.0 - underside * material.params_a.w;
    platform_rgb *= mix(0.72, 1.0, top_face);

    base_rgb = mix(base_rgb, platform_rgb, platform_mask);
    emissive_rgb += material.emissive.rgb
        * border_glow
        * (0.18 + 0.28 * top_face)
        * platform_mask;
    emissive_rgb += material.secondary.rgb
        * (center_line * 0.012 + scan_band * 0.03)
        * inset_mask
        * top_face
        * platform_mask;

    pbr_input.material.perceptual_roughness = mix(
        pbr_input.material.perceptual_roughness,
        mix(0.96, 0.68, top_face),
        platform_mask,
    );
    pbr_input.material.clearcoat = mix(
        pbr_input.material.clearcoat,
        0.01 + 0.03 * inset_mask * top_face,
        platform_mask,
    );
    pbr_input.material.clearcoat_perceptual_roughness = mix(
        pbr_input.material.clearcoat_perceptual_roughness,
        mix(0.82, 0.58, top_face),
        platform_mask,
    );

    base_rgb += material.accent.rgb * silhouette * 0.03;
    base_rgb = mix(base_rgb, material.atmosphere.rgb, distance_t * (0.12 + 0.12 * side_face));
    emissive_rgb += sky_reflection * fresnel * 0.02;

    pbr_input.material.base_color = vec4<f32>(base_rgb, pbr_input.material.base_color.a);
    pbr_input.material.emissive = vec4<f32>(emissive_rgb, 1.0);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);

    let compressed_platform = out.color.rgb / (vec3<f32>(1.0) + out.color.rgb * material.params_c.w);
    let platform_target = mix(platform_rgb * 0.85, compressed_platform, 0.58 + 0.18 * top_face);
    out.color = vec4<f32>(mix(out.color.rgb, platform_target, platform_mask * 0.72), out.color.a);

    let atmospheric_tint = mix(material.atmosphere.rgb, sky_reflection, 0.35 + 0.65 * top_face);
    out.color = vec4<f32>(
        mix(out.color.rgb, atmospheric_tint, distance_t * (0.14 + 0.12 * side_face)),
        out.color.a,
    );
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);

    return out;
}
