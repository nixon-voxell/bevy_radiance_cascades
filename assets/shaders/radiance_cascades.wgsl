#import bevy_render::maths::{PI_2, HALF_PI}

const QUARTER_PI: f32 = HALF_PI * 0.5;
const MAX_RAYMARCH: u32 = 100;
const EPSILON: f32 = 4.88e-04;

struct Probe {
    width: u32,
    start: f32,
    range: f32,
}

@group(0) @binding(0) var<uniform> probe: Probe;
@group(0) @binding(1) var tex_main: texture_2d<f32>;
@group(0) @binding(2) var tex_dist_field: texture_2d<f32>;
@group(0) @binding(3) var tex_radiance_cascades_source: texture_2d<f32>;
@group(0) @binding(4) var tex_radiance_cascades_destination: texture_storage_2d<rgba32float, write>;

@compute
@workgroup_size(8, 8, 1)
fn radiance_cascades(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>
) {
    let base_coord = global_id.xy;

    let probe_x = base_coord.x % probe.width;
    let probe_y = base_coord.y % probe.width;

    let ray_index = probe_x + probe_y * probe.width;
    let ray_count = probe.width * probe.width;

    var ray_angle = f32(ray_index) / f32(ray_count) * PI_2;
    // Rotate 45 degrees
    ray_angle += QUARTER_PI;
    let ray_dir = vec2<f32>(cos(ray_angle), sin(ray_angle));

    // Center coordinate of the probe grid
    var probe_coord = base_coord / probe.width * probe.width;
    probe_coord += probe.width / 2;

    let origin = vec2<f32>(probe_coord) + ray_dir * probe.start;

    let color = raymarch(origin, ray_dir, probe.range);

    textureStore(
        tex_radiance_cascades_destination,
        base_coord,
        // vec4<f32>(distance(vec2f(base_coord), vec2f(probe_coord))/f32(probe.width))
        // vec4<f32>(
        //     f32(probe_x)/f32(probe.width),
        //     f32(probe_y)/f32(probe.width),
        //     0, 0
        // )
        color
    );
}

fn raymarch(_origin: vec2<f32>, ray_dir: vec2<f32>, range: f32) -> vec4<f32> {
    var color = vec4<f32>(0.0);
    var origin = _origin;
    var covered_range = 0.0;

    let dimensions = vec2<f32>(textureDimensions(tex_main));

    for (var r = 0u; r < MAX_RAYMARCH; r++) {
        if (
            covered_range >= range ||
            any(origin > dimensions) ||
            any(origin < vec2<f32>(0.0))
        ) {
            break;
        }

        var dist = textureLoad(tex_dist_field, vec2<u32>(round(origin)), 0).r;
        if (dist < EPSILON) {
            color = textureLoad(tex_main, vec2<u32>(round(origin)), 0) - 1.0;
        }

        origin += ray_dir * dist;
        covered_range += dist;
    }

    return color;
}

fn merge(base_coord: vec2<u32>) {
    let prev_width = probe.width * 2;
    for (var p = 0u; p < 4u; p++) {
        
    }
}
