#import bevy_render::maths::{PI_2, HALF_PI}
#import "shaders/radiance_probe.wgsl"::Probe;

const QUARTER_PI: f32 = HALF_PI * 0.5;
const MAX_RAYMARCH: u32 = 32;
const EPSILON: f32 = 4.88e-04;

@group(0) @binding(0) var<uniform> probe: Probe;
@group(0) @binding(1) var tex_main: texture_2d<f32>;
@group(0) @binding(2) var tex_dist_field: texture_2d<f32>;
@group(0) @binding(3) var tex_radiance_cascades_source: texture_2d<f32>;
@group(0) @binding(4) var tex_radiance_cascades_destination: texture_storage_2d<rgba16float, write>;

@compute
@workgroup_size(8, 8, 1)
fn radiance_cascades(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>
) {
    let base_coord = global_id.xy;

    // Coordinate inside the probe grid
    let probe_texel = base_coord % probe.width;

    let ray_index = probe_texel.x + probe_texel.y * probe.width;
    let ray_count = probe.width * probe.width;

    var ray_angle = f32(ray_index) / f32(ray_count) * PI_2;
    // Rotate 45 degrees
    ray_angle += QUARTER_PI;
    let ray_dir = vec2<f32>(cos(ray_angle), sin(ray_angle));

    // Coordinate of cell in probe grid
    let probe_cell = base_coord / probe.width;
    // Start coordinate of the probe grid (in texture space)
    let probe_coord = probe_cell * probe.width;

    // Center coordinate of the probe grid
    var probe_coord_center = probe_coord + probe.width / 2;
    // TODO: Investigate
    // 0.5 is added as probe width is always an even number
    let origin = vec2<f32>(probe_coord_center) + ray_dir * probe.start;

    var color = raymarch(origin, ray_dir, probe.range);

#ifdef MERGE
    // TODO: Factor in transparency.
    if (color.a != 1.0) {
        color += merge(probe_cell, probe_coord, ray_index);
    }
#endif

    textureStore(
        tex_radiance_cascades_destination,
        base_coord,
        color
    );
}

fn raymarch(origin: vec2<f32>, ray_dir: vec2<f32>, range: f32) -> vec4<f32> {
    var color = vec4<f32>(0.0);
    var position = origin;
    var covered_range = 0.0;

    let dimensions = vec2<f32>(textureDimensions(tex_main));

    for (var r = 0u; r < MAX_RAYMARCH; r++) {
        if (
            covered_range >= range ||
            any(position > dimensions) ||
            any(position < vec2<f32>(0.0))
        ) {
            break;
        }

        var dist = textureLoad(tex_dist_field, vec2<u32>(round(position)), 0).r;
        position += ray_dir * dist;
        if (dist < EPSILON) {
            position += ray_dir * 1.0;
            color = textureLoad(tex_main, vec2<u32>(round(position)), 0);

            // Treat values from -1.0 ~ 1.0 as no light
            // This way, we can handle both negative and postive light
            let color_sign = sign(color);
            let color_abs = abs(color);

            color.r = color_sign.r * max(color_abs.r - 1.0, 0.0);
            color.g = color_sign.g * max(color_abs.g - 1.0, 0.0);
            color.b = color_sign.b * max(color_abs.b - 1.0, 0.0);

            // TODO: Factor in transparency.
            color.a = 1.0;
            break;
        }

        covered_range += dist;
    }

    return color;
}

fn merge(probe_cell: vec2<u32>, probe_coord: vec2<u32>, ray_index: u32) -> vec4<f32> {
    let dimensions = textureDimensions(tex_radiance_cascades_source);
    let prev_width = probe.width * 2;

    var TL = vec4<f32>(0.0);
    var TR = vec4<f32>(0.0);
    var BL = vec4<f32>(0.0);
    var BR = vec4<f32>(0.0);

    let prev_ray_index_start = ray_index * 4;
    for (var p: u32 = 0; p < 4; p++) {
        let prev_ray_index = prev_ray_index_start + p;

        let offset_coord = vec2<u32>(
            prev_ray_index % prev_width,
            prev_ray_index / prev_width
        );

        TL += fetch_cascade(
            probe_cell,
            vec2<u32>(0, 0),
            offset_coord,
            dimensions,
            prev_width
        );
        TR += fetch_cascade(
            probe_cell,
            vec2<u32>(1, 0),
            offset_coord,
            dimensions,
            prev_width
        );
        BL += fetch_cascade(
            probe_cell,
            vec2<u32>(0, 1),
            offset_coord,
            dimensions,
            prev_width
        );
        BR += fetch_cascade(
            probe_cell,
            vec2<u32>(1, 1),
            offset_coord,
            dimensions,
            prev_width
        );
    }

    let weight = 0.25 + (
        (vec2<f32>(probe_coord) - vec2<f32>(probe_cell / 2 * prev_width)) / f32(prev_width)
    ) * 0.5;

    return mix(mix(TL, TR, weight.x), mix(BL, BR, weight.x), weight.y) * 0.25;
}

fn fetch_cascade(
    // Current probe's start coordinate
    probe_cell: vec2<u32>,
    probe_offset: vec2<u32>,
    offset_coord: vec2<u32>,
    dimensions: vec2<u32>,
    prev_width: u32,
) -> vec4<f32> {
    let prev_probe_cell = (probe_cell / 2 + probe_offset) * prev_width;
    let prev_probe_coord = prev_probe_cell + offset_coord;

    if (any(prev_probe_coord >= dimensions)) {
        return vec4<f32>(0.0);
    }

    return textureLoad(tex_radiance_cascades_source, prev_probe_coord, 0);
}
