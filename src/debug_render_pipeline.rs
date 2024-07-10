use bevy::{
    core_pipeline::{
        core_2d::graph::{Core2d, Node2d},
        fullscreen_vertex_shader::fullscreen_shader_vertex_state,
    },
    prelude::*,
    render::{
        render_graph::{RenderGraphApp, RenderLabel, ViewNode, ViewNodeRunner},
        render_resource::{
            binding_types::texture_2d, BindGroupEntries, BindGroupLayout, BindGroupLayoutEntries,
            CachedRenderPipelineId, ColorTargetState, ColorWrites, FragmentState, PipelineCache,
            RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor,
            ShaderStages, TextureFormat, TextureSampleType,
        },
        renderer::RenderDevice,
        view::ViewTarget,
        RenderApp,
    },
};

pub struct DebugRenderPipelinePlugin;

impl Plugin for DebugRenderPipelinePlugin {
    fn build(&self, app: &mut App) {
        // We need to get the render app from the main app
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_render_graph_node::<ViewNodeRunner<DebugRenderPipelineNode>>(
                Core2d,
                DebugRenderPipelineLabel,
            )
            .add_render_graph_edges(
                Core2d,
                // Specify the node ordering.
                // This will automatically create all required node edges to enforce the given ordering.
                (
                    Node2d::ContrastAdaptiveSharpening,
                    DebugRenderPipelineLabel,
                    Node2d::EndMainPassPostProcessing,
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        // We need to get the render app from the main app
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            // Initialize the pipeline
            .init_resource::<DebugRenderPipeline>();
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct DebugRenderPipelineLabel;

#[derive(Default)]
pub struct DebugRenderPipelineNode;

impl ViewNode for DebugRenderPipelineNode {
    type ViewQuery = (&'static ViewTarget, &'static crate::jfa::JfaPrepassTextures);

    fn run<'w>(
        &self,
        _graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        (view_target, debug_texture): bevy::ecs::query::QueryItem<'w, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        // Get the pipeline resource that contains the global data we need
        // to create the render pipeline
        let debug_render_pipeline = world.resource::<DebugRenderPipeline>();

        // The pipeline cache is a cache of all previously created pipelines.
        // It is required to avoid creating a new pipeline each frame,
        // which is expensive due to shader compilation.
        let pipeline_cache = world.resource::<PipelineCache>();

        // Get the pipeline from the cache
        let Some(pipeline) = pipeline_cache.get_render_pipeline(debug_render_pipeline.pipeline_id)
        else {
            return Ok(());
        };

        let post_process = view_target.post_process_write();

        // The bind_group gets created each frame.
        //
        // Normally, you would create a bind_group in the Queue set,
        // but this doesn't work with the post_process_write().
        // The reason it doesn't work is because each post_process_write will alternate the source/destination.
        // The only way to have the correct source/destination for the bind_group
        // is to make sure you get it during the node execution.
        let bind_group = render_context.render_device().create_bind_group(
            "post_process_bind_group",
            &debug_render_pipeline.layout,
            // It's important for this to match the BindGroupLayout defined in the PostProcessPipeline
            &BindGroupEntries::single(
                // Make sure to use the source view
                &debug_texture.main_texture().default_view,
            ),
        );

        // Begin the render pass
        let mut render_pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("post_process_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                // We need to specify the post process destination view here
                // to make sure we write to the appropriate texture.
                view: post_process.destination,
                resolve_target: None,
                ops: default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        // This is mostly just wgpu boilerplate for drawing a fullscreen triangle,
        // using the pipeline/bind_group created above
        render_pass.set_render_pipeline(pipeline);
        // By passing in the index of the post process settings on this view, we ensure
        // that in the event that multiple settings were sent to the GPU (as would be the
        // case with multiple cameras), we use the correct one.
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..3, 0..1);

        Ok(())
    }
}

// This contains global data used by the render pipeline. This will be created once on startup.
#[derive(Resource)]
struct DebugRenderPipeline {
    layout: BindGroupLayout,
    pipeline_id: CachedRenderPipelineId,
}

impl FromWorld for DebugRenderPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();

        // We need to define the bind group layout used for our pipeline
        let layout = render_device.create_bind_group_layout(
            "post_process_bind_group_layout",
            &BindGroupLayoutEntries::single(
                // The layout entries will only be visible in the fragment stage
                ShaderStages::FRAGMENT,
                // The screen texture
                texture_2d(TextureSampleType::Uint {}),
            ),
        );

        // Get the shader handle
        let shader = world.load_asset("shaders/debug_render_pipeline.wgsl");

        let pipeline_id = world
            .resource_mut::<PipelineCache>()
            // This will add the pipeline to the cache and queue it's creation
            .queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("post_process_pipeline".into()),
                layout: vec![layout.clone()],
                // This will setup a fullscreen triangle for the vertex state
                vertex: fullscreen_shader_vertex_state(),
                fragment: Some(FragmentState {
                    shader,
                    shader_defs: vec![],
                    // Make sure this matches the entry point of your shader.
                    // It can be anything as long as it matches here and in the shader.
                    entry_point: "fragment".into(),
                    targets: vec![Some(ColorTargetState {
                        format: TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                // All of the following properties are not important for this effect so just use the default values.
                // This struct doesn't have the Default trait implemented because not all field can have a default value.
                primitive: default(),
                depth_stencil: None,
                multisample: default(),
                push_constant_ranges: vec![],
            });

        Self {
            layout,
            pipeline_id,
        }
    }
}
