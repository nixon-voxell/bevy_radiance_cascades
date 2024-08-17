use bevy::{
    // color::palettes::css,
    core_pipeline::{bloom::BloomSettings, smaa::SmaaSettings, tonemapping::Tonemapping},
    prelude::*,
    sprite::Mesh2dHandle,
    window::PrimaryWindow,
};

mod debug_render_pipeline;
mod jfa;
mod mask2d;
mod math_util;
mod radiance_cascades;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(mask2d::Mask2dPrepassPlugin::<ColorMaterial>::default())
        .add_plugins(jfa::JfaPrepassPlugin)
        .add_plugins(radiance_cascades::RadianceCascadesPlugin)
        // .add_plugins(debug_render_pipeline::DebugRenderPipelinePlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, follow_mouse)
        .add_systems(Update, cascade_settings)
        // .add_systems(Update, draw_radiance_cascade_rays)
        .run();
}

#[derive(Component)]
pub struct Marked;

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
                clear_color: Color::BLACK.into(),
                hdr: true,
                ..default()
            },
            tonemapping: Tonemapping::AcesFitted,
            ..default()
        },
        jfa::JfaPrepass,
        mask2d::Mask2dPrepass,
        radiance_cascades::RadianceCascadesConfig::new(1, 128.0),
        // BloomSettings::default(),
        // SmaaSettings::default(),
    ));

    // rect
    commands.spawn((
        ColorMesh2dBundle {
            mesh: Mesh2dHandle(meshes.add(Rectangle {
                half_size: Vec2::new(20.0, 60.0),
            })),
            material: materials.add(Color::linear_rgba(0.0, 0.0, 10.0, 0.5)),
            transform: Transform::from_xyz(-100.0, 100.0, 0.0),
            ..default()
        },
        mask2d::Mask2d,
    ));
    commands.spawn((
        ColorMesh2dBundle {
            mesh: Mesh2dHandle(meshes.add(Circle { radius: 50.0 })),
            material: materials.add(Color::linear_rgba(10.0, 0.0, 0.0, 0.5)),
            transform: Transform::from_xyz(0.0, 0.0, 0.1),
            ..default()
        },
        mask2d::Mask2d,
        Marked,
    ));
}

fn follow_mouse(
    q_window: Query<&Window, With<PrimaryWindow>>,
    mut q_marked: Query<&mut Transform, With<Marked>>,
) {
    let Ok(window) = q_window.get_single() else {
        return;
    };

    let Some(mut cursor_position) = window.cursor_position() else {
        return;
    };
    let width = window.width();
    let height = window.height();

    cursor_position.y = -cursor_position.y;
    cursor_position += Vec2::new(-width, height) * 0.5;

    for mut transform in q_marked.iter_mut() {
        transform.translation.x = cursor_position.x;
        transform.translation.y = cursor_position.y;
    }
}

fn cascade_settings(
    mut q_cascade: Query<&mut radiance_cascades::RadianceCascadesConfig>,
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
) {
    const SPEED: f32 = 8.0;
    let Ok(mut config) = q_cascade.get_single_mut() else {
        return;
    };

    let speed = SPEED
        * match keyboard.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
            true => 32.0,
            false => 1.0,
        };

    if keyboard.pressed(KeyCode::ArrowUp) {
        let interval = config.get_interval();
        config.set_interval(interval + time.delta_seconds() * speed);
    }
    if keyboard.pressed(KeyCode::ArrowDown) {
        let interval = config.get_interval();
        config.set_interval(interval - time.delta_seconds() * speed);
    }
}
