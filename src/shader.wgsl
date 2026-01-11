struct Vertex {
    @location(0) position: vec3<f32>,
}

struct CallBox {
    @location(1) start_time: vec2<u32>,
    @location(2) end_time: vec2<u32>,
    @location(3) method_id: u32,
    @location(4) depth: u32,
}

struct GCBox {
    @location(1) time: vec2<u32>,
}

struct CameraUniform {
    view_proj: mat4x4<f32>,
    base_time: vec2<u32>, // x: lo, y: hi
    max_depth: u32,
    num_threads: u32,
}

struct ThreadInfo {
    lane_id: u32,
}

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(0) @binding(1)
var<uniform> thread_info: ThreadInfo;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

fn sub64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    if (a.x < b.x) {
        let lo = a.x + 0x80000000u - b.x;
        let hi = a.y - 1 - b.y;
        return vec2<u32>(lo, hi);
    } else {
        let lo = a.x - b.x;
        let hi = a.y - b.y;
        return vec2<u32>(lo, hi);
    }
}

fn u64tof32(v: vec2<u32>) -> f32 {
    return f32(v.y) * 2147483648.0 + f32(v.x);
}

fn get_color(method_id: u32) -> vec4<f32> {
    let m = method_id;
    let r = f32((m * 123u) % 255u) / 255.0;
    let g = f32((m * 456u) % 255u) / 255.0;
    let b = f32((m * 789u) % 255u) / 255.0;
    return vec4<f32>(r, g, b, 1.0);
}

@vertex
fn vs_main(
    v: Vertex,
    call: CallBox,
) -> VertexOutput {
    var end_time: vec2<u32>;
    if (call.end_time.y == 0xffffffffu) {
        end_time = vec2<u32>(0, 0);
    } else {
        end_time = sub64(camera.base_time, call.end_time);
    }
    let start_time = sub64(camera.base_time, call.start_time);

    let x = select(start_time, end_time, v.position.x > 0.5);

    let world_pos = vec3<f32>(
        u64tof32(x) / 500000000.0,
        (f32(call.depth) + v.position.y) / f32(camera.max_depth),
        (f32(thread_info.lane_id) + v.position.z) / f32(camera.num_threads),
    );

    var out: VertexOutput;
    out.color = get_color(call.method_id);
    out.clip_position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}

struct GCVertex {
    @location(0) position: vec2<f32>,
}

@vertex
fn vs_gc(
    v: GCVertex,
    gc: GCBox,
) -> VertexOutput {
    let time = sub64(camera.base_time, gc.time);

    let world_pos = vec3<f32>(
        u64tof32(time) / 500000000.0,
        v.position.x,
        v.position.y,
    );

    var out: VertexOutput;
    out.color = vec4<f32>(1.0, 0.5, 0.0, 0.1);
    out.clip_position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    return out;
}
