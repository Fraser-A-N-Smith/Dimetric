// Multiplies the world by the light buffer and scales the result to the output.
//
// Also where the pixel-art path is honoured: the world is drawn at a fixed
// internal resolution and this pass scales it by a whole number with nearest
// sampling, so every source pixel becomes exactly the same number of screen
// pixels. Scaling sprites individually instead is what makes some of them a
// pixel wider than their neighbours.

struct Settings {
    // Ambient light, multiplied into everything the lights do not reach.
    ambient: vec4<f32>,
    // x is 0 when the light buffer should be ignored entirely; the rest is
    // unused. A vec4 rather than a float and a pad, because vec3 carries
    // 16-byte alignment in WGSL and the obvious Rust mirror of it is the wrong
    // size -- which the validator catches, but only at the first draw.
    lighting: vec4<f32>,
};

@group(0) @binding(0) var world_texture: texture_2d<f32>;
@group(0) @binding(1) var world_sampler: sampler;
@group(0) @binding(2) var light_texture: texture_2d<f32>;
@group(0) @binding(3) var<uniform> settings: Settings;

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32) -> VertexOut {
    // One oversized triangle rather than two triangles: no seam down the
    // diagonal, and one fewer vertex to think about.
    let uv = vec2<f32>(
        f32((vertex << 1u) & 2u),
        f32(vertex & 2u),
    );
    var out: VertexOut;
    out.clip_position = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    // Flip v: texture space runs downward, clip space runs upward.
    out.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let world = textureSample(world_texture, world_sampler, in.uv);

    // Opaque, always. The world texture is cleared transparent so sprites blend
    // against it correctly, but the composite is the finished picture: a
    // captured frame should be an image, not an image with holes in it. Where
    // nothing was drawn the colour is already black, which is the background.
    var lit = world.rgb;
    if (settings.lighting.x != 0.0) {
        let light = textureSample(light_texture, world_sampler, in.uv);
        lit = world.rgb * (settings.ambient.rgb + light.rgb);
    }
    return vec4<f32>(lit, 1.0);
}
