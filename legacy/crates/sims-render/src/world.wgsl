@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_2d<f32>;
@group(0) @binding(2) var nearest_sampler: sampler;
struct VertexIn {
 @location(0) position:vec3<f32>,
 @location(1) uv:vec2<f32>,
 @location(2) tint:vec4<f32>,
 @location(3) depth_scale:f32,
};
struct Varying {
 @builtin(position) position:vec4<f32>,
 @location(0) uv:vec2<f32>,
 @location(1) tint:vec4<f32>,
 @location(2) depth_scale:f32,
};
@vertex fn vs(v:VertexIn)->Varying {
 var out:Varying;
 out.position=vec4(v.position,1.0);out.uv=v.uv;out.tint=v.tint;out.depth_scale=v.depth_scale;
 return out;
}
struct Output { @location(0) color:vec4<f32>, @builtin(frag_depth) depth:f32 };
@fragment fn fs(v:Varying)->Output {
 let color=textureSample(color_tex,nearest_sampler,v.uv)*v.tint;
 if color.a<=0.0 {discard;}
 var out:Output;
 out.color=color;
 out.depth=v.position.z+(1.0-textureSample(depth_tex,nearest_sampler,v.uv).r)*v.depth_scale;
 return out;
}
