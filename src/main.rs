use bevy::{
    core_pipeline::{bloom::BloomSettings, smaa::SmaaSettings},
    prelude::*,
    sprite::Mesh2dHandle,
};

mod debug_render_pipeline;
mod jfa;
mod mask2d;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, jfa::JfaPrepassPlugin))
        .add_plugins(debug_render_pipeline::DebugRenderPipelinePlugin)
        .add_plugins(mask2d::Mask2dPrepassPlugin::<ColorMaterial>::default())
        .add_systems(Startup, setup)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    // camera
    commands.spawn((
        Camera2dBundle {
            transform: Transform::from_translation(Vec3::new(0.0, 0.0, 5.0))
                .looking_at(Vec3::default(), Vec3::Y),
            camera: Camera {
                clear_color: Color::NONE.into(),
                hdr: true,
                ..default()
            },
            ..default()
        },
        BloomSettings::default(),
        SmaaSettings::default(),
        jfa::JfaPrepass,
        mask2d::Mask2dPrepass,
    ));

    // rect
    commands
        .spawn(ColorMesh2dBundle {
            mesh: Mesh2dHandle(meshes.add(Rectangle {
                half_size: Vec2::new(60.0, 20.0),
            })),
            material: materials.add(Color::srgba(0.0, 0.0, 0.3, 0.5)),
            transform: Transform::from_xyz(-50.0, 100.0, 0.0),
            ..default()
        })
        .insert(mask2d::Mask2d);
    commands
        .spawn(ColorMesh2dBundle {
            mesh: Mesh2dHandle(meshes.add(Circle { radius: 100.0 })),
            material: materials.add(Color::srgba(0.0, 2.0, 0.0, 0.5)),
            transform: Transform::from_xyz(50.0, 0.0, 0.1),
            ..default()
        })
        .insert(mask2d::Mask2d);
}
