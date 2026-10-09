// Instanced sprites. One draw per (atlas, blend mode, shader) run.
//
// The quad is generated from the vertex index rather than read from a buffer:
// there is only ever one quad shape, and not binding a vertex buffer is one
// less thing to keep in sync per batch.

struct Camera {
    view_proj: mat4x4<f32>,
    // Clip-space units per world unit of sprite size. The projection moves a
    // sprite's position; this sizes its quad.
    pixel_scale: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct Instance {
    @location(0) center: vec2<f32>,
    // Pixels added to the centre after the projection, in the same units and
    // through the same scale as the quad below. A glyph's advance along a line
    // of text goes here: the label's anchor is world geometry and projects,
    // where the next letter sits is typography and must not. Zero for a
    // sprite, a tile and a panel, whose positions are entirely world geometry.
    @location(1) screen_offset: vec2<f32>,
    @location(2) size: vec2<f32>,
    // cos and sin of the sprite's rotation, computed on the CPU from the
    // engine's fixed-point tables so that what is drawn matches what the
    // simulation decided.
    @location(3) rotation: vec2<f32>,
    @location(4) uv_min: vec2<f32>,
    @location(5) uv_max: vec2<f32>,
    @location(6) color: vec4<f32>,
    // 1.0 when the light buffer must not apply to this sprite, 0.0 otherwise.
    // Read only by `fs_mask`; the colour pass ignores it.
    @location(7) unlit: f32,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) unlit: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, instance: Instance) -> VertexOut {
    // Two triangles: 0,1,2 and 2,1,3.
    let corner = vec2<f32>(
        f32((vertex & 1u) != 0u),
        f32((vertex & 2u) != 0u),
    );
    let local = (corner - vec2<f32>(0.5, 0.5)) * instance.size;
    let rotated = vec2<f32>(
        local.x * instance.rotation.x - local.y * instance.rotation.y,
        local.x * instance.rotation.y + local.y * instance.rotation.x,
    );

    // The centre goes through the projection; the quad is added in screen
    // space afterwards. Under the shear that keeps the sprite upright and only
    // its position projected, which is what isometric artwork expects.
    var out: VertexOut;
    let center = camera.view_proj * vec4<f32>(instance.center, 0.0, 1.0);
    let screen = (rotated + instance.screen_offset) * camera.pixel_scale.xy;
    out.clip_position = center + vec4<f32>(screen, 0.0, 0.0);
    out.uv = mix(instance.uv_min, instance.uv_max, corner);
    out.color = instance.color;
    out.unlit = instance.unlit;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(atlas_texture, atlas_sampler, in.uv) * in.color;
}

// Which pixels the light must not touch, into the mask target.
//
// Red carries the answer and alpha carries the sprite's real coverage, which is
// what blends it: `unlit * coverage + dst * (1 - coverage)`. Keeping the two
// apart is forced rather than tidy — a mask stored in alpha would be both the
// value and the factor, and a fully opaque *lit* sprite wanting to write zero
// over an unlit one would write `0 * 0 + dst * 1` and change nothing.
//
// A second pass over the same instances in the same sort order, and that order
// is the point: a lit figure drawn in front of an unlit range marker writes
// zero over it, so the figure still dims. A mask built out of order would let
// whatever was marked unlit leak its exemption onto everything drawn over it.
@fragment
fn fs_mask(in: VertexOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(atlas_texture, atlas_sampler, in.uv).a * in.color.a;
    return vec4<f32>(in.unlit, 0.0, 0.0, coverage);
}
