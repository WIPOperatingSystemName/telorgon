#version 450

struct GpuSpatial { vec4 local_to_view_0; vec4 local_to_view_1; };
struct GpuBoxInstance {
    vec4 rect; vec4 radii; vec4 border_widths;
    uvec4 fill_border_t_r_b; uvec4 border_l_spatial_clip_flags;
    float opacity; uint reserved_0; uint reserved_1; uint reserved_2;
    vec4 outline; vec4 shadow_0; vec4 shadow_1; uvec4 outline_shadow_colors;
};
layout(set=0,binding=0,std140) uniform ViewBlock { vec4 clip_from_view_0; vec4 clip_from_view_1; vec4 clip_from_view_2; vec4 clip_from_view_3; vec4 view_size_scale; vec4 target_size_origin; vec4 render_size_inverse; uvec4 epoch_flags; vec4 placement_clip_rects[2]; vec4 placement_clip_radii[2]; } view_data;
layout(set=1,binding=0,std430) readonly buffer SpatialBlock { GpuSpatial values[]; } spatials;
layout(set=1,binding=2,std430) readonly buffer DrawIndexBlock { uint values[]; } draw_indices;
layout(set=2,binding=0,std430) readonly buffer BoxBlock { GpuBoxInstance values[]; } boxes;
layout(location=0) noperspective out vec2 local_position;
layout(location=1) noperspective out vec2 view_position;
layout(location=2) flat out uint instance_slot;
const vec2 QUAD[4]=vec2[4](vec2(0,0),vec2(1,0),vec2(0,1),vec2(1,1));

void include_shadow(in vec4 shadow, inout vec4 extent) {
    float reach=max(0.0,shadow.w+shadow.z*2.0);
    extent.x=max(extent.x,reach-shadow.x);
    extent.y=max(extent.y,reach-shadow.y);
    extent.z=max(extent.z,reach+shadow.x);
    extent.w=max(extent.w,reach+shadow.y);
}

void main(){
    instance_slot=draw_indices.values[gl_InstanceIndex];
    GpuBoxInstance item=boxes.values[instance_slot];
    float outline_extent=max(0.0,item.outline.x+item.outline.y);
    vec4 extent=vec4(outline_extent);
    uint shadow_count=item.outline_shadow_colors.w;
    if(shadow_count>0u)include_shadow(item.shadow_0,extent);
    if(shadow_count>1u)include_shadow(item.shadow_1,extent);
    vec2 low=-extent.xy;
    vec2 high=item.rect.zw+extent.zw;
    local_position=mix(low,high,QUAD[gl_VertexIndex]);
    vec2 local=item.rect.xy+local_position;
    GpuSpatial spatial=spatials.values[item.border_l_spatial_clip_flags.y];
    view_position=vec2(dot(spatial.local_to_view_0.xyz,vec3(local,1)),dot(spatial.local_to_view_1.xyz,vec3(local,1)));
    vec4 p=vec4(view_position,0,1);
    gl_Position=vec4(dot(view_data.clip_from_view_0,p),dot(view_data.clip_from_view_1,p),dot(view_data.clip_from_view_2,p),dot(view_data.clip_from_view_3,p));
}
