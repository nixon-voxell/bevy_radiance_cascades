#import bevy_render::maths::{PI, HALF_PI}

struct Probe {
    width: u32,
    interval: f32,
}

@group(0) @binding(0) var<uniform> probe_width: Probe;
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

    let main = textureLoad(tex_main, base_coord/4, 0);

    textureStore(
        tex_radiance_cascades_destination,
        base_coord,
        main - 1.0
    );
}

fn raymarch(origin: vec2<f32>, theta: f32) -> vec4<f32> {
    return vec4<f32>(0.0);
}
