@group(0) @binding(0) var mask_texture: texture_2d<u32>;
@group(0) @binding(1) var jfa_texture: texture_storage_2d<rg16uint, write>;

@compute
@workgroup_size(8, 8, 1)
fn jfa_mask(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>
) {
    let base_coordinates = vec2<u32>(global_id.xy);

    let mask = textureLoad(mask_texture, base_coordinates, 0).r;

    if mask == 1u {
        textureStore(jfa_texture, base_coordinates, vec4<u32>(base_coordinates, 0, 0));
    } else {
        // Set to a far distance
        let far_coordinate = textureDimensions(jfa_texture) * 3u;
        textureStore(jfa_texture, base_coordinates, vec4<u32>(far_coordinate, 0, 0));
    }
}
