// Multiplies the world by the light buffer and scales the result to the output.
//
// Also where the pixel-art path is honoured: the world is drawn at a fixed
// internal resolution and this pass scales the whole of it at once, so every
// source pixel becomes exactly the same number of screen pixels. Scaling
// sprites individually instead is what makes some of them a pixel wider than
// their neighbours.
//
// `world_sampler` is nearest at a whole scale and linear otherwise, chosen by
// the bind group the host sets — see `RenderSettings::present_linear`. At a
// whole scale nearest reproduces the frame exactly; at 0.9 it would never read
// one source row and column in ten, which on a thin letter is a missing stroke
// rather than a missing pixel.

struct Settings {
    // Ambient light, multiplied into everything the lights do not reach.
    ambient: vec4<f32>,
    // x is 0 when the light buffer should be ignored entirely; y is 0 when
    // nothing in the frame asked to be left out of the multiply, and the mask
    // must not be read. The rest is unused. A vec4 rather than a float and a
    // pad, because vec3 carries 16-byte alignment in WGSL and the obvious Rust
    // mirror of it is the wrong size -- which the validator catches, but only
    // at the first draw.
    lighting: vec4<f32>,
};

@group(0) @binding(0) var world_texture: texture_2d<f32>;
@group(0) @binding(1) var world_sampler: sampler;
@group(0) @binding(2) var light_texture: texture_2d<f32>;
@group(0) @binding(3) var<uniform> settings: Settings;
// The UI layer, drawn through the canvas projection into its own target.
@group(0) @binding(4) var ui_texture: texture_2d<f32>;
// Red is how much of each pixel came from drawing the light must not touch —
// a health bar over a head, a damage number, the cells an ability can reach.
// Written by `sprite.wgsl`'s `fs_mask` in the same order as the colour, so a
// lit sprite drawn in front clears what is behind it.
@group(0) @binding(5) var mask_texture: texture_2d<f32>;

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
        // Per pixel, between "the light decides" and "leave it alone". A health
        // bar over a monster's head, a damage number and the cells an ability
        // can reach are world drawing — they sit on the board and move with the
        // camera — but they are read rather than looked at, so they must not
        // dim with the room. Mixing rather than branching so a half-covered
        // edge pixel is half exempt instead of picking a side.
        var exempt = 0.0;
        if (settings.lighting.y != 0.0) {
            exempt = textureSample(mask_texture, world_sampler, in.uv).r;
        }
        lit = world.rgb * mix(settings.ambient.rgb + light.rgb, vec3<f32>(1.0), exempt);
    }
    // UI last and unlit. A health bar does not get darker when the player
    // walks into a shadow, so it is blended over the finished picture rather
    // than drawn into the world and multiplied by the light buffer.
    let ui = textureSample(ui_texture, world_sampler, in.uv);
    let out = lit * (1.0 - ui.a) + ui.rgb;
    return vec4<f32>(out, 1.0);
}
