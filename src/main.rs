use bevy::{
    // color::palettes::css,
    core_pipeline::{bloom::BloomSettings, smaa::SmaaSettings},
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
        radiance_cascades::RadianceCascadesConfig::new(1, 2.0),
    ));

    // rect
    commands.spawn((
        ColorMesh2dBundle {
            mesh: Mesh2dHandle(meshes.add(Rectangle {
                half_size: Vec2::new(20.0, 60.0),
            })),
            material: materials.add(Color::linear_rgba(0.0, 0.0, 4.0, 0.5)),
            transform: Transform::from_xyz(-100.0, 100.0, 0.0),
            ..default()
        },
        mask2d::Mask2d,
    ));
    commands.spawn((
        ColorMesh2dBundle {
            mesh: Mesh2dHandle(meshes.add(Circle { radius: 50.0 })),
            material: materials.add(Color::linear_rgba(0.0, 0.0, -4.0, 0.5)),
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

    for mut tranfsorm in q_marked.iter_mut() {
        tranfsorm.translation.x = cursor_position.x;
        tranfsorm.translation.y = cursor_position.y;
    }
}

// fn draw_radiance_cascade_rays(q_window: Query<&Window, With<PrimaryWindow>>, mut gizmos: Gizmos) {
//     let Ok(window) = q_window.get_single() else {
//         return;
//     };

//     let Some(mut cursor_position) = window.cursor_position() else {
//         return;
//     };
//     let width = window.width();
//     let height = window.height();

//     cursor_position.y = -cursor_position.y;
//     cursor_position += Vec2::new(-width, height) * 0.5;
//     let diagonal = f32::sqrt(width * width + height * height);

//     let interval0 = 4.0;
//     let resolution_factor = 1;

//     /*
//     The sum of all intervals can be achieved using geometric sequence:
//     https://saylordotorg.github.io/text_intermediate-algebra/s12-03-geometric-sequences-and-series.html

//     Formula: Sn = a1(1−r^n)/(1−r)
//     Where:
//     - Sn: sum of all intervals
//     - a1: first interval
//     -  r: factor (4 as each interval increases its length by 4 every new cascade)
//     -  n: number of cascades

//     The goal here is to find n such that Sn < diagonal.
//     let x = diagonal

//     Factoring in the numbers:
//     x > Sn
//     x > a1(1−4^n)/-3

//     Rearranging the equation:
//     -3(x) > a1(1−4^n)
//     -3(x)/a1 > 1−4^n
//     4^n > 1 + 3(x)/a1
//     n > log4(1 + 3(x)/a1)
//     */
//     // Ceil is used becaues n should be greater than the value we get.
//     let cascade_count = f32::log(1.0 + 3.0 * diagonal / interval0, 4.0).ceil() as usize;

//     const RAINBOW: [Srgba; 7] = [
//         css::RED,
//         css::ORANGE,
//         css::YELLOW,
//         css::GREEN,
//         css::BLUE,
//         css::VIOLET,
//         css::PURPLE,
//     ];

//     for c in 0..cascade_count {
//         let ray_count = 1 << ((c + resolution_factor) * 2);

//         let start = interval0 * (1.0 - f32::powi(4.0, c as i32)) / -3.0;
//         let length = interval0 * f32::powi(4.0, c as i32);

//         for r in 0..ray_count {
//             let mut theta = r as f32 / ray_count as f32 * std::f32::consts::TAU;
//             // Add 45 degree
//             theta += std::f32::consts::PI * 0.25;
//             let delta = Vec2::new(f32::cos(theta), f32::sin(theta));
//             let origin = cursor_position + delta * start;
//             // gizmos.line_2d(origin, origin + delta * length, RAINBOW[c]);
//         }
//     }
// }
