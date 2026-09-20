#version 450
layout(set=0,binding=0,std140) uniform ViewBlock { vec4 clip_from_view_0; vec4 clip_from_view_1; vec4 clip_from_view_2; vec4 clip_from_view_3; vec4 view_size_scale; vec4 target_size_origin; vec4 render_size_inverse; uvec4 epoch_flags; vec4 placement_clip_rects[2]; vec4 placement_clip_radii[2]; } view_data;
float placement_coverage(){
    float amount=1.0;
    for(int i=0;i<2;i++){
        vec4 rect=view_data.placement_clip_rects[i];
        if(rect.z<0.0)continue;
        bool inverted=(view_data.epoch_flags.w&(1u<<uint(i+1)))!=0u;
        if(rect.z<=0.0||rect.w<=0.0){if(inverted)continue;return 0.0;}
        vec2 p=gl_FragCoord.xy-rect.xy;
        vec2 half_size=rect.zw*.5;
        vec4 radii=view_data.placement_clip_radii[i];
        float radius=p.x<half_size.x?(p.y<half_size.y?radii.x:radii.w):(p.y<half_size.y?radii.y:radii.z);
        radius=clamp(radius,0.0,min(half_size.x,half_size.y));
        vec2 q=abs(p-half_size)-(half_size-vec2(radius));
        float d=length(max(q,vec2(0)))+min(max(q.x,q.y),0.0)-radius;
        float coverage=clamp(.5-d,0.0,1.0);
        amount=min(amount,inverted?1.0-coverage:coverage);
    }
    return amount;
}

struct GpuClip { vec4 view_bounds; vec4 local_rect; vec4 local_from_view_0; vec4 local_from_view_1; vec4 radii; vec4 mask_uv_from_view_0; vec4 mask_uv_from_view_1; uvec4 mode_mask_flags; };
struct GpuGlyphInstance { vec4 rect; vec4 uv_texels; uvec4 color_spatial_clip_page; float opacity; uint flags; uvec2 reserved; };
layout(set=1,binding=1,std430) readonly buffer ClipBlock { GpuClip values[]; } clips;
layout(set=2,binding=0,std430) readonly buffer GlyphBlock { GpuGlyphInstance values[]; } glyphs;
layout(set=3,binding=0) uniform sampler2D atlas_texture;
layout(location=0) noperspective in vec2 uv_texels;layout(location=1) noperspective in vec2 view_position;layout(location=2) flat in uint instance_slot;layout(location=0) out vec4 output_color;
vec4 unpack_srgba(uint p){return vec4(float(p&255u),float((p>>8u)&255u),float((p>>16u)&255u),float((p>>24u)&255u))/255.0;}
vec3 srgb_decode(vec3 v){bvec3 low=lessThanEqual(v,vec3(.04045));return mix(pow((v+.055)/1.055,vec3(2.4)),v/12.92,low);}
// Convert local signed distance to output pixels using the analytic edge normal.
// An L1 derivative width (fwidth) widens the AA band by sqrt(2) at 45 degrees.
float rounded_pixel_width(vec2 p,vec2 size,vec4 radii){
    vec2 dx=dFdx(p);
    vec2 dy=dFdy(p);
    vec2 half_size=size*.5;
    float radius=p.x<half_size.x?(p.y<half_size.y?radii.x:radii.w):(p.y<half_size.y?radii.y:radii.z);
    radius=clamp(radius,0.0,min(half_size.x,half_size.y));
    vec2 delta=p-half_size;
    vec2 q=abs(delta)-(half_size-vec2(radius));
    vec2 outside=max(q,vec2(0));
    vec2 normal=dot(outside,outside)>0.0?normalize(outside):(q.x>q.y?vec2(1,0):vec2(0,1));
    normal*=sign(delta);
    return max(length(vec2(dot(normal,dx),dot(normal,dy))),1e-4);
}
float clip_coverage(uint slot,vec2 p){
    if(slot==0xffffffffu)return 1.0;
    GpuClip c=clips.values[slot];
    vec2 local=p-c.view_bounds.xy;
    // Keep rounded coverage derivatives ahead of per-fragment bounds rejection.
    if(c.mode_mask_flags.x!=2u)return all(greaterThanEqual(local,vec2(0)))&&all(lessThanEqual(local,c.view_bounds.zw))?1.0:0.0;
    vec2 half_size=c.view_bounds.zw*.5;
    float radius=local.x<half_size.x?(local.y<half_size.y?c.radii.x:c.radii.w):(local.y<half_size.y?c.radii.y:c.radii.z);
    radius=clamp(radius,0.0,min(half_size.x,half_size.y));
    vec2 q=abs(local-half_size)-(half_size-vec2(radius));
    float d=length(max(q,vec2(0)))+min(max(q.x,q.y),0.0)-radius;
    // Match the analytic box edge rather than rounding coverage to a boolean.
    return clamp(.5-d/rounded_pixel_width(local,c.view_bounds.zw,c.radii),0.0,1.0);
}
void main(){
    GpuGlyphInstance item=glyphs.values[instance_slot];
    // Sample before clipping so neighboring helper invocations supply valid derivatives.
    float coverage=texture(atlas_texture,uv_texels/vec2(textureSize(atlas_texture,0))).r;
    float clip_amount=clip_coverage(item.color_spatial_clip_page.z,view_position);
    float placement_amount=placement_coverage()*clip_amount;
    if(placement_amount<=0.0)discard;
    vec4 color=unpack_srgba(item.color_spatial_clip_page.x);
    float a=color.a*coverage*clamp(item.opacity,0,1);
    output_color=vec4(srgb_decode(color.rgb)*a,a)*placement_amount;
}
