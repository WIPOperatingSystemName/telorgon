#version 450
layout(set=0,binding=0,std140) uniform ViewBlock { vec4 clip_from_view_0; vec4 clip_from_view_1; vec4 clip_from_view_2; vec4 clip_from_view_3; vec4 view_size_scale; vec4 target_size_origin; vec4 render_size_inverse; uvec4 epoch_flags; vec4 placement_clip_rects[2]; vec4 placement_clip_radii[2]; } view_data;
float placement_coverage(){
    float amount=1.0;
    for(int i=0;i<2;i++){
        if(i==1&&(view_data.epoch_flags.w&8u)!=0u)continue;
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
        if(all(lessThanEqual(radii,vec4(0)))){
            vec2 overlap=clamp(min(p+vec2(.5),rect.zw)-max(p-vec2(.5),vec2(0)),vec2(0),vec2(1));
            coverage=overlap.x*overlap.y;
        }
        amount=min(amount,inverted?1.0-coverage:coverage);
    }
    return amount;
}

// Slot 1 is a coverage denominator only while painting an isolated frame interior.
float normalization_coverage(){
    if((view_data.epoch_flags.w&8u)==0u)return 1.0;
    vec4 rect=view_data.placement_clip_rects[1];
    if(rect.z<=0.0||rect.w<=0.0)return 0.0;
    vec2 p=gl_FragCoord.xy-rect.xy;
    vec2 half_size=rect.zw*.5;
    vec4 radii=view_data.placement_clip_radii[1];
    float radius=p.x<half_size.x?(p.y<half_size.y?radii.x:radii.w):(p.y<half_size.y?radii.y:radii.z);
    radius=clamp(radius,0.0,min(half_size.x,half_size.y));
    vec2 q=abs(p-half_size)-(half_size-vec2(radius));
    float d=length(max(q,vec2(0)))+min(max(q.x,q.y),0.0)-radius;
    if(all(lessThanEqual(radii,vec4(0)))){
        vec2 overlap=clamp(min(p+vec2(.5),rect.zw)-max(p-vec2(.5),vec2(0)),vec2(0),vec2(1));
        return overlap.x*overlap.y;
    }
    return clamp(.5-d,0.0,1.0);
}

// One physical pixel of paint clearance; hit testing and layout are unchanged.
float interior_paint_coverage(){
    if((view_data.epoch_flags.w&16u)==0u)return 1.0;
    vec4 rect=view_data.placement_clip_rects[1]+vec4(1,1,-2,-2);
    if(rect.z<=0.0||rect.w<=0.0)return 0.0;
    vec2 p=gl_FragCoord.xy-rect.xy;
    vec2 half_size=rect.zw*.5;
    vec4 radii=max(view_data.placement_clip_radii[1]-vec4(1),vec4(0));
    float radius=p.x<half_size.x?(p.y<half_size.y?radii.x:radii.w):(p.y<half_size.y?radii.y:radii.z);
    radius=clamp(radius,0.0,min(half_size.x,half_size.y));
    vec2 q=abs(p-half_size)-(half_size-vec2(radius));
    float d=length(max(q,vec2(0)))+min(max(q.x,q.y),0.0)-radius;
    if(all(lessThanEqual(radii,vec4(0)))){
        vec2 overlap=clamp(min(p+vec2(.5),rect.zw)-max(p-vec2(.5),vec2(0)),vec2(0),vec2(1));
        return overlap.x*overlap.y;
    }
    return clamp(.5-d,0.0,1.0);
}

struct GpuClip { vec4 view_bounds; vec4 local_rect; vec4 local_from_view_0; vec4 local_from_view_1; vec4 radii; vec4 mask_uv_from_view_0; vec4 mask_uv_from_view_1; uvec4 mode_mask_flags; };
struct GpuMaterialInstance { vec4 rect; uvec4 params_spatial_clip; float opacity; uint material_variant; uint flags; uint reserved; uvec4 resource_range_reserved; };
layout(set=1,binding=1,std430) readonly buffer ClipBlock { GpuClip values[]; } clips;
layout(set=2,binding=0,std430) readonly buffer MaterialBlock { GpuMaterialInstance values[]; } materials;
layout(set=2,binding=1,std430) readonly buffer ParameterBlock { uint values[]; } parameters;
layout(location=0) noperspective in vec2 unit_position;layout(location=1) noperspective in vec2 view_position;layout(location=2) flat in uint instance_slot;layout(location=0) out vec4 output_color;
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
    if(c.mode_mask_flags.x!=2u){
        vec2 footprint=max(abs(dFdx(local))+abs(dFdy(local)),vec2(1e-4));
        vec2 overlap=max(min(local+footprint*.5,c.view_bounds.zw)-max(local-footprint*.5,vec2(0)),vec2(0));
        vec2 amount=clamp(overlap/footprint,vec2(0),vec2(1));
        return amount.x*amount.y;
    }
    vec2 half_size=c.view_bounds.zw*.5;
    float radius=local.x<half_size.x?(local.y<half_size.y?c.radii.x:c.radii.w):(local.y<half_size.y?c.radii.y:c.radii.z);
    radius=clamp(radius,0.0,min(half_size.x,half_size.y));
    vec2 q=abs(local-half_size)-(half_size-vec2(radius));
    float d=length(max(q,vec2(0)))+min(max(q.x,q.y),0.0)-radius;
    // Match the analytic box edge rather than rounding coverage to a boolean.
    return clamp(.5-d/rounded_pixel_width(local,c.view_bounds.zw,c.radii),0.0,1.0);
}
void main(){
    GpuMaterialInstance item=materials.values[instance_slot];
    float clip_amount=clip_coverage(item.params_spatial_clip.w,view_position);
    clip_amount=min(clip_amount,interior_paint_coverage());
    float normalizer=normalization_coverage();
    float placement_amount=clamp(placement_coverage()*clip_amount/max(normalizer,1e-6),0.0,1.0);
    if(placement_amount<=0.0||normalizer<=0.0)discard;
    float t=item.material_variant==1u?unit_position.x:(item.material_variant==2u?unit_position.y:0.0);
    vec4 first=unpack_srgba(parameters.values[item.params_spatial_clip.x]);
    vec4 second=unpack_srgba(parameters.values[item.params_spatial_clip.x+1u]);
    vec4 color=mix(first,second,t);
    float a=color.a*clamp(item.opacity,0,1);
    output_color=vec4(srgb_decode(color.rgb)*a,a)*placement_amount;
}
