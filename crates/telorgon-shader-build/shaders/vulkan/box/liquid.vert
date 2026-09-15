#version 450
struct GpuSpatial { vec4 local_to_view_0; vec4 local_to_view_1; };
struct GpuMaterialInstance { vec4 rect; uvec4 params_spatial_clip; float opacity; uint material_variant; uint flags; uint reserved; uvec4 resource_range_reserved; };
layout(set=0,binding=0,std140) uniform ViewBlock { vec4 clip_from_view_0; vec4 clip_from_view_1; vec4 clip_from_view_2; vec4 clip_from_view_3; vec4 view_size_scale; vec4 target_size_origin; vec4 render_size_inverse; uvec4 epoch_flags; vec4 placement_clip_rects[2]; vec4 placement_clip_radii[2]; } view_data;
layout(set=1,binding=0,std430) readonly buffer SpatialBlock { GpuSpatial values[]; } spatials;
layout(set=1,binding=2,std430) readonly buffer DrawIndexBlock { uint values[]; } draw_indices;
layout(set=2,binding=0,std430) readonly buffer MaterialBlock { GpuMaterialInstance values[]; } materials;
layout(set=2,binding=1,std430) readonly buffer ParameterBlock { uint values[]; } parameters;
layout(location=3) flat out vec4 panel_rect;
layout(location=4) flat out vec4 lens_radii;
layout(location=5) flat out vec4 lens_uv_bevel_refraction;
layout(location=6) flat out vec4 lens_lighting;
layout(location=7) flat out vec4 lens_tint;
layout(location=8) flat out vec2 lens_blend;
vec4 lens_parameter(uint offset) {
    return uintBitsToFloat(uvec4(parameters.values[offset], parameters.values[offset+1u], parameters.values[offset+2u], parameters.values[offset+3u]));
}
layout(location=0) noperspective out vec2 unit_position;layout(location=1) noperspective out vec2 view_position;layout(location=2) flat out uint instance_slot;
const vec2 QUAD[4]=vec2[4](vec2(0,0),vec2(1,0),vec2(0,1),vec2(1,1));
void main(){instance_slot=draw_indices.values[gl_InstanceIndex];GpuMaterialInstance item=materials.values[instance_slot];panel_rect=item.rect;
uint parameter_offset=item.params_spatial_clip.x;
lens_radii=lens_parameter(parameter_offset);
lens_uv_bevel_refraction=lens_parameter(parameter_offset+4u);
lens_lighting=lens_parameter(parameter_offset+8u);
lens_tint=lens_parameter(parameter_offset+12u);
float bevel=1.0/lens_uv_bevel_refraction.z;
float softness=max(uintBitsToFloat(parameters.values[parameter_offset+16u]),0.0);
float outer=max(bevel,min(bevel+softness,0.5*min(item.rect.z,item.rect.w)));
// Inverse outer fade width and its subtle contribution; computed per vertex, not pixel.
lens_blend=vec2(1.0/outer,0.25*(outer-bevel)/outer);
unit_position=QUAD[gl_VertexIndex];vec2 local=item.rect.xy+unit_position*item.rect.zw;GpuSpatial spatial=spatials.values[item.params_spatial_clip.z];view_position=vec2(dot(spatial.local_to_view_0.xyz,vec3(local,1)),dot(spatial.local_to_view_1.xyz,vec3(local,1)));vec4 p=vec4(view_position,0,1);gl_Position=vec4(dot(view_data.clip_from_view_0,p),dot(view_data.clip_from_view_1,p),dot(view_data.clip_from_view_2,p),dot(view_data.clip_from_view_3,p));}
