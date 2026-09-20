#version 450
struct GpuSpatial { vec4 local_to_view_0; vec4 local_to_view_1; };
struct GpuGlyphInstance { vec4 rect; vec4 uv_texels; uvec4 color_spatial_clip_page; float opacity; uint flags; uvec2 reserved; };
layout(set=0,binding=0,std140) uniform ViewBlock { vec4 clip_from_view_0; vec4 clip_from_view_1; vec4 clip_from_view_2; vec4 clip_from_view_3; vec4 view_size_scale; vec4 target_size_origin; vec4 render_size_inverse; uvec4 epoch_flags; vec4 placement_clip_rects[2]; vec4 placement_clip_radii[2]; } view_data;
layout(set=1,binding=0,std430) readonly buffer SpatialBlock { GpuSpatial values[]; } spatials;
layout(set=1,binding=2,std430) readonly buffer DrawIndexBlock { uint values[]; } draw_indices;
layout(set=2,binding=0,std430) readonly buffer GlyphBlock { GpuGlyphInstance values[]; } glyphs;
layout(location=0) noperspective out vec2 uv_texels;
layout(location=1) noperspective out vec2 view_position;
layout(location=2) flat out uint instance_slot;
const vec2 QUAD[4]=vec2[4](vec2(0,0),vec2(1,0),vec2(0,1),vec2(1,1));
void main(){instance_slot=draw_indices.values[gl_InstanceIndex];GpuGlyphInstance item=glyphs.values[instance_slot];vec2 unit=QUAD[gl_VertexIndex];uv_texels=item.uv_texels.xy+unit*item.uv_texels.zw;vec2 local=item.rect.xy+unit*item.rect.zw;GpuSpatial spatial=spatials.values[item.color_spatial_clip_page.y];view_position=vec2(dot(spatial.local_to_view_0.xyz,vec3(local,1)),dot(spatial.local_to_view_1.xyz,vec3(local,1)));vec4 p=vec4(view_position,0,1);gl_Position=vec4(dot(view_data.clip_from_view_0,p),dot(view_data.clip_from_view_1,p),dot(view_data.clip_from_view_2,p),dot(view_data.clip_from_view_3,p));}
