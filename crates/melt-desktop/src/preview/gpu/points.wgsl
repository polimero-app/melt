struct Uniforms {
    mvp: mat4x4<f32>, light: vec4<f32>, frame: vec4<f32>, point: vec4<f32>, depth: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
struct VertexOut { @builtin(position) position: vec4<f32>, @location(0) @interpolate(flat) shade: f32 }
@vertex fn vs(@builtin(vertex_index) vertex: u32, @location(0) projected: vec3<f32>) -> VertexOut {
    let corners = array<vec2<f32>,6>(vec2(-1.0,-1.0),vec2(1.0,-1.0),vec2(-1.0,1.0),vec2(-1.0,1.0),vec2(1.0,-1.0),vec2(1.0,1.0));
    let center = floor((projected.xy-u.depth.xy)*u.point.x+u.point.yz)+vec2(0.5);
    let pixel = center + corners[vertex] * (u.frame.z+0.5);
    var out: VertexOut;
    out.shade = clamp((projected.z-u.depth.z)/u.depth.w,0.0,1.0);
    out.position = vec4(pixel.x/u.frame.x*2.0-1.0,1.0-pixel.y/u.frame.y*2.0,1.0-out.shade,1.0);
    return out;
}
@fragment fn fs(in: VertexOut) -> @location(0) vec4<f32> {
    let rgb = floor(vec3(30.0,145.0,185.0)+in.shade*vec3(35.0,80.0,55.0))/255.0;
    return vec4(rgb,1.0);
}
