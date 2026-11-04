struct Uniforms {
    mvp: mat4x4<f32>, light: vec4<f32>, frame: vec4<f32>, point: vec4<f32>, depth: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: Uniforms;
struct VertexOut { @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32> }
@vertex fn vs(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = u.mvp * vec4(position,1.0);
    out.position.z = (out.position.z + out.position.w) * 0.5;
    out.normal = normal;
    return out;
}
@fragment fn fs(in: VertexOut) -> @location(0) vec4<f32> {
    var diffuse = 0.0;
    if dot(in.normal,in.normal) > 0.000000000001 { diffuse = abs(dot(normalize(in.normal),u.light.xyz)); }
    let rgb = floor(clamp(vec3(30.0,145.0,185.0) + diffuse * vec3(35.0,80.0,55.0),vec3(0.0),vec3(255.0))) / 255.0;
    return vec4(rgb,1.0);
}
