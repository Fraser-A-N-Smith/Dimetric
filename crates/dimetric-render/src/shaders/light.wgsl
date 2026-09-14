// Additive light accumulation.
//
// One quad per light covering its radius, blended additively into a buffer the
// composite pass multiplies the world by. Cheap, which is the point: a light
// costs one quad and a little arithmetic, so a spell can be a light source
// without anyone having to think about it.

struct Camera {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct Instance {
    @location(0) center: vec2<f32>,
    @location(1) radius: f32,
    @location(2) energy: f32,
    @location(3) color: vec4<f32>,
    // Cone direction as cos/sin, and the cosine of the cone's half-width.
    // A half-width cosine of -1 accepts every direction, which is how a radial
    // light is expressed without a second pipeline.
    @location(4) direction: vec2<f32>,
    @location(5) cone_cos: f32,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) offset: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) energy: f32,
    @location(3) direction: vec2<f32>,
    @location(4) cone_cos: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, instance: Instance) -> VertexOut {
    let corner = vec2<f32>(
        f32((vertex & 1u) != 0u),
        f32((vertex & 2u) != 0u),
    );
    let local = (corner - vec2<f32>(0.5, 0.5)) * (instance.radius * 2.0);

    var out: VertexOut;
    out.clip_position = camera.view_proj * vec4<f32>(instance.center + local, 0.0, 1.0);
    // Carried in units of the radius, so the fragment shader needs no uniform
    // to know how far out it is.
    out.offset = (corner - vec2<f32>(0.5, 0.5)) * 2.0;
    out.color = instance.color;
    out.energy = instance.energy;
    out.direction = instance.direction;
    out.cone_cos = instance.cone_cos;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let distance = length(in.offset);
    if (distance > 1.0) {
        discard;
    }

    // Smooth inverse-square-ish falloff. Not physically motivated: it is the
    // curve that looks like a torch in a dark room, which is what the shape is
    // for.
    var falloff = 1.0 - distance;
    falloff = falloff * falloff;

    if (in.cone_cos > -1.0) {
        let toward = select(normalize(in.offset), in.direction, distance < 0.0001);
        let alignment = dot(toward, in.direction);
        if (alignment < in.cone_cos) {
            discard;
        }
        // Soften the cone's edge over the last tenth, so it does not alias into
        // a hard wedge.
        let edge = clamp((alignment - in.cone_cos) / 0.1, 0.0, 1.0);
        falloff = falloff * edge;
    }

    return vec4<f32>(in.color.rgb * in.color.a * in.energy * falloff, 1.0);
}
