#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct WorldPostProcessSettings {
    params_a: vec4<f32>,
    params_b: vec4<f32>,
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: WorldPostProcessSettings;

fn saturate(value: f32) -> f32 {
    return clamp(value, 0.0, 1.0);
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn sample_source(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(source_texture, source_sampler, uv, 0.0).rgb;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let center = sample_source(in.uv);
    let texel = 1.0 / vec2<f32>(textureDimensions(source_texture));
    let glow_radius = texel * (1.6 + settings.params_b.z * 1.2);

    let blur = (
        sample_source(in.uv + vec2<f32>(glow_radius.x, 0.0))
            + sample_source(in.uv - vec2<f32>(glow_radius.x, 0.0))
            + sample_source(in.uv + vec2<f32>(0.0, glow_radius.y))
            + sample_source(in.uv - vec2<f32>(0.0, glow_radius.y))
            + sample_source(in.uv + glow_radius)
            + sample_source(in.uv - glow_radius)
            + sample_source(in.uv + vec2<f32>(glow_radius.x, -glow_radius.y))
            + sample_source(in.uv + vec2<f32>(-glow_radius.x, glow_radius.y))
    ) / 8.0;

    let highlight_mask = smoothstep(settings.params_a.z, 1.0, luminance(blur));
    let radial = in.uv * 2.0 - 1.0;
    let radial_length = length(radial);
    let chroma_dir = radial * settings.params_a.w * (0.65 + settings.params_b.z * 0.35);
    let chroma_color = vec3<f32>(
        sample_source(in.uv + chroma_dir).r,
        center.g,
        sample_source(in.uv - chroma_dir).b,
    );

    var color = center;
    color = mix(color, chroma_color, highlight_mask * settings.params_b.y);
    color += max(blur - center, vec3<f32>(0.0)) * highlight_mask * settings.params_b.x;

    let vignette = 1.0 - smoothstep(0.55, 1.2, radial_length) * settings.params_a.x;
    color *= vignette;

    let gray = vec3<f32>(luminance(color));
    color = mix(gray, color, 1.0 + settings.params_b.w);
    color = (color - 0.5) * settings.params_a.y + 0.5;

    return vec4<f32>(color, 1.0);
}
