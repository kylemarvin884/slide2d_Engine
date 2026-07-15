struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) texture_coordinate: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) texture_coordinate: vec2<f32>,
};

@group(0) @binding(0) var object_texture: texture_2d<f32>;
@group(0) @binding(1) var object_sampler: sampler;

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.texture_coordinate = input.texture_coordinate;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(object_texture, object_sampler, input.texture_coordinate);
}
