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
layout(set=1,binding=1,std430) readonly buffer ClipBlock { GpuClip values[]; } clips;
layout(location=0) noperspective in vec2 unit_position;layout(location=1) noperspective in vec2 view_position;layout(location=2) flat in uint lens_clip;layout(location=0) out vec4 output_color;
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

// Original Telorgon implementation. See docs/LIQUID_GLASS.md for the reference study
// and outline-matched lens derivation. One owned backdrop, no feedback sampling.
layout(set=3,binding=0) uniform sampler2D backdrop;
layout(set=3,binding=1) uniform sampler2D sharp_backdrop;
layout(location=3) flat in vec4 panel_rect;
layout(location=4) flat in vec4 lens_radii;
layout(location=5) flat in vec4 lens_uv_bevel_refraction;
layout(location=6) flat in vec2 lens_lighting;
layout(location=7) flat in vec4 lens_tint;
layout(location=8) flat in vec4 lens_blend;
layout(location=9) flat in float lens_opacity;

// Static optical tuning: tan(4 degrees). Zero disables swirl; negative reverses it.
// Kept in the offline shader so this adds no uniform traffic or animation clock.
const float EDGE_TWIST_TANGENT=0.06992681194;
vec2 twist_refraction(vec2 slope, float edge_weight) {
    float twist=EDGE_TWIST_TANGENT*edge_weight;
    // Exact rotation by atan(twist), preserving displacement magnitude without trig.
    return (slope+vec2(-slope.y,slope.x)*twist)*inversesqrt(1.0+twist*twist);
}

// Rounded-window distance controls the outer band; its interior crease is smoothed.
// Optical direction is separate because the exact SDF gradient jumps on diagonal ties.
float lens_falloff(vec2 p, vec2 half_size, float radius) {
    vec2 q=abs(p)-half_size+radius;
    float nearest=max(q.x,q.y);
    // Exact interior fast path: no corner overlap can pull these pixels into the band.
    if(nearest<=min(0.0,radius-lens_blend.z))return 0.0;
    // Inside the corner's circle center, exact SDF insets have a diagonal medial
    // crease. Smooth that interior max too: smoothing direction alone leaves a
    // kink in bend magnitude (and in the sharp/blur blend).
    if(nearest<0.0) {
        float depth_sum=-(q.x+q.y);
        float limit=lens_blend.w;
        // Stop smoothing at the inner fade boundary so the clear center stays clear.
        float remaining=2.0*limit-radius-0.5*depth_sum;
        float width=limit*depth_sum/(limit+depth_sum)*smoothstep(0.0,1.0,remaining*(2.0*lens_blend.x));
        if(width>0.0) {
            float overlap=max(1.0-abs(q.x-q.y)/width,0.0);
            nearest+=0.25*width*overlap*overlap;
        }
    }
    float distance=length(max(q,vec2(0)))+min(nearest,0.0)-radius;
    float t=clamp(1.0+distance*lens_uv_bevel_refraction.z,0.0,1.0);
    // Quintic endpoints have zero first and second derivatives. Squaring puts
    // most bending at the rim and leaves a soft tail into the blurred interior.
    float edge=t*t*t*(t*(6.0*t-15.0)+10.0);
    float core=edge*edge;
    if(lens_blend.y==0.0)return core;
    float outer_t=clamp(1.0+distance*lens_blend.x,0.0,1.0);
    float tail=outer_t*outer_t*outer_t*(outer_t*(6.0*outer_t-15.0)+10.0);
    // Preserve the strong rim and add only a gentle tail beyond the core band.
    return core+(1.0-core)*lens_blend.y*tail*tail;
}
vec2 lens_direction(vec2 p, vec2 half_size, float radius) {
    float inverse_bevel=lens_blend.x;
    // Enlarging only the direction radius makes adjacent edge influences overlap
    // before their nearest-edge normals could switch. It does not alter the outline.
    // Work in bevel units, reusing the CPU reciprocal instead of dividing per pixel.
    float optical_radius=min(max(radius*inverse_bevel,1.0),min(half_size.x,half_size.y)*inverse_bevel);
    vec2 v=max((abs(p)-half_size)*inverse_bevel+optical_radius,vec2(0));
    const float softness=0.25;
    // C1 activation at each straight-to-corner join; no normalize(0) at the center.
    v=v*v/(v+softness);
    return sign(p)*v*inversesqrt(max(dot(v,v),1e-8));
}
void main() {
    // Derivatives used by generic clipping execute before any varying control flow.
    float coverage=placement_coverage()*clip_coverage(lens_clip,view_position);
    if(coverage<=0.0) discard;
    vec2 half_size=panel_rect.zw*0.5;
    vec2 p=(unit_position-0.5)*panel_rect.zw;
    float radius=p.x<0.0?(p.y<0.0?lens_radii.x:lens_radii.w):(p.y<0.0?lens_radii.y:lens_radii.z);
    radius=clamp(radius,0.0,min(half_size.x,half_size.y));
    float t=lens_falloff(p,half_size,radius);
    vec2 bend=vec2(0);
    vec2 separation=vec2(0);
    float reflection=0.0;
    if(t>0.0) {
        vec2 slope=lens_direction(p,half_size,radius)*t;
        vec2 refracted_slope=twist_refraction(slope,t);
        bend=-refracted_slope*lens_uv_bevel_refraction.w;
        separation=refracted_slope*lens_lighting.x;
        if(lens_lighting.y>0.0) {
            float grazing=1.0-sqrt(max(1.0-dot(slope,slope),0.0));
            grazing*=grazing;
            reflection=grazing*grazing*lens_lighting.y;
        }
    }
    vec2 uv=(panel_rect.xy+unit_position*panel_rect.zw+bend)*lens_uv_bevel_refraction.xy;
    // The interior stays blurred. Refraction approaches the full-resolution sharp source
    // continuously at the rim, without magnifying the blur kernel into coarse blocks.
    vec3 color=textureLod(backdrop,uv,0.0).rgb;
    if(t>0.0) {
        // The central sample supplies all channels without dispersion, or green with it.
        vec3 sharp=textureLod(sharp_backdrop,uv,0.0).rgb;
        if(lens_lighting.x>0.0) {
            vec2 delta=separation*lens_uv_bevel_refraction.xy;
            sharp.r=textureLod(sharp_backdrop,uv+delta,0.0).r;
            sharp.b=textureLod(sharp_backdrop,uv-delta,0.0).b;
        }
        color=mix(color,sharp,t);
    }
    color=color*lens_tint.w+lens_tint.rgb;
    color=mix(color,vec3(1),reflection*0.2);
    float alpha=lens_opacity*coverage;
    output_color=vec4(color*alpha,alpha);
}
