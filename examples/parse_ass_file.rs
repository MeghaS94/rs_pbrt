extern crate getopts;
extern crate num_cpus;
extern crate pbrt;
// pest
extern crate pest;
#[macro_use]
extern crate pest_derive;

// parser
use pest::Parser;

// getopts
use getopts::Options;
// pbrt
use pbrt::accelerators::bvh::{BVHAccel, SplitMethod};
use pbrt::cameras::perspective::PerspectiveCamera;
use pbrt::core::camera::Camera;
use pbrt::core::film::Film;
use pbrt::core::filter::Filter;
use pbrt::core::geometry::{Bounds2f, Bounds2i, Point2f, Point2i, Point3f};
use pbrt::core::integrator::SamplerIntegrator;
use pbrt::core::light::Light;
use pbrt::core::medium::MediumInterface;
use pbrt::core::paramset::ParamSet;
use pbrt::core::pbrt::Float;
use pbrt::core::primitive::{GeometricPrimitive, Primitive, TransformedPrimitive};
use pbrt::core::sampler::Sampler;
use pbrt::core::scene::Scene;
use pbrt::core::transform::{AnimatedTransform, Matrix4x4, Transform};
use pbrt::filters::gaussian::GaussianFilter;
use pbrt::integrators::path::PathIntegrator;
use pbrt::integrators::render;
use pbrt::samplers::sobol::SobolSampler;
// std
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

#[derive(Parser)]
#[grammar = "../examples/ass.pest"]
struct AssParser;

// TransformSet (copied from api.rs)

#[derive(Debug, Default, Copy, Clone)]
pub struct TransformSet {
    pub t: [Transform; 2],
}

impl TransformSet {
    pub fn is_animated(&self) -> bool {
        // for (int i = 0; i < MaxTransforms - 1; ++i)
        //     if (t[i] != t[i + 1]) return true;
        // return false;

        // we have only 2 transforms
        if self.t[0] != self.t[1] {
            true
        } else {
            false
        }
    }
}

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [options]", program);
    print!("{}", opts.usage(&brief));
}

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

fn print_version(program: &str) {
    println!("{} {}", program, VERSION);
}

fn strip_comments(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let v: Vec<&str> = input.lines().map(str::trim).collect();
    for line in v {
        if let Some(_found) = line.find('#') {
            let v2: Vec<&str> = line.split('#').collect();
            let stripped_line = v2[0];
            output.push_str(stripped_line);
            output.push_str("\n");
        } else {
            output.push_str(line);
            output.push_str("\n");
        }
    }
    output
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();
    let mut opts = Options::new();
    opts.optflag("h", "help", "print this help menu");
    opts.optopt("i", "", "parse an input file", "FILE");
    opts.optflag("v", "version", "print version number");
    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => panic!(f.to_string()),
    };
    if matches.opt_present("h") {
        print_usage(&program, opts);
        return;
    } else if matches.opt_present("i") {
        // default values
        let mut node_name: String = String::from(""); // no default name
        let mut filter_name: String = String::from("box");
        let mut filter_width: Float = 2.0;
        let mut render_camera: String = String::from(""); // no default name
        let mut camera_name: String = String::from("perspective");
        let mut fov: Float = 90.0; // read persp_camera.fov
        let mut xres: i32 = 1280; // read options.xres
        let mut yres: i32 = 720; // read options.yres
        let mut max_depth: i32 = 5; // read options.GI_total_depth
                                    // input (.ass) file
        let infile = matches.opt_str("i");
        let primitives: Vec<Arc<Primitive + Sync + Send>> = Vec::new();
        let lights: Vec<Arc<Light + Sync + Send>> = Vec::new();
        match infile {
            Some(x) => {
                println!("FILE = {}", x);
                let f = File::open(x.clone()).unwrap();
                let ip: &Path = Path::new(x.as_str());
                if ip.is_relative() {
                    let cp: PathBuf = env::current_dir().unwrap();
                    let pb: PathBuf = cp.join(ip);
                    let search_directory: &Path = pb.as_path().parent().unwrap();
                    println!("search_directory is {}", search_directory.display());
                }
                let mut reader = BufReader::new(f);
                let mut str_buf: String = String::default();
                let num_bytes = reader.read_to_string(&mut str_buf);
                if num_bytes.is_ok() {
                    let n_bytes = num_bytes.unwrap();
                    println!("{} bytes read", n_bytes);
                }
                // parser
                let pairs =
                    AssParser::parse(Rule::ass, &str_buf).unwrap_or_else(|e| panic!("{}", e));
                // let tokens: Vec<_> = pairs.flatten().tokens().collect();
                // println!("{} pairs", tokens.len());
                for pair in pairs {
                    let span = pair.clone().into_span();
                    // println!("Rule:    {:?}", pair.as_rule());
                    // println!("Span:    {:?}", span);
                    // println!("Text:    {}", span.as_str());
                    for inner_pair in pair.into_inner() {
                        match inner_pair.as_rule() {
                            Rule::ident => {
                                let node_type = inner_pair.clone().into_span().as_str();
                                print!("{} {{", node_type);
                                let stripped = strip_comments(span.as_str());
                                let mut iter = stripped.split_whitespace();
                                loop {
                                    if let Some(next) = iter.next() {
                                        if next != String::from("}") {
                                            if next == String::from("name") {
                                                if let Some(name) = iter.next() {
                                                    node_name = name.to_string();
                                                    print!(" {} {} ", next, node_name);
                                                }
                                            }
                                            if node_type == String::from("options") {
                                                if next == String::from("xres") {
                                                    if let Some(xres_str) = iter.next() {
                                                        xres = i32::from_str(xres_str).unwrap();
                                                        print!("\n xres {} ", xres);
                                                    }
                                                } else if next == String::from("yres") {
                                                    if let Some(yres_str) = iter.next() {
                                                        yres = i32::from_str(yres_str).unwrap();
                                                        print!("\n yres {} ", yres);
                                                    }
                                                } else if next == String::from("camera") {
                                                    if let Some(camera_str) = iter.next() {
                                                        // strip surrounding double quotes
                                                        let v: Vec<&str> = camera_str.split('"').collect();
                                                        render_camera = v[1].to_string();
                                                        print!("\n camera {:?} ", render_camera);
                                                    }
                                                } else if next == String::from("GI_total_depth") {
                                                    if let Some(max_depth_str) = iter.next() {
                                                        max_depth =
                                                            i32::from_str(max_depth_str).unwrap();
                                                        print!("\n GI_total_depth {} ", max_depth);
                                                    }
                                                }
                                            } else if node_type == String::from("persp_camera")
                                                && node_name == render_camera
                                            {
                                                camera_name = String::from("perspective");
                                                if next == String::from("fov") {
                                                    if let Some(fov_str) = iter.next() {
                                                        fov = f32::from_str(fov_str).unwrap();
                                                        print!("\n fov {} ", fov);
                                                    }
                                                }
                                            } else if node_type == String::from("gaussian_filter") {
                                                filter_name = String::from("gaussian");
                                                if next == String::from("width") {
                                                    if let Some(filter_width_str) = iter.next() {
                                                        filter_width =
                                                            f32::from_str(filter_width_str)
                                                                .unwrap();
                                                        print!("\n filter_width {} ", filter_width);
                                                    }
                                                }
                                            } else if node_type == String::from("polymesh") {
                                                if next == String::from("vlist") {
                                                    // parameter_name: vlist
                                                    // <num_elements>
                                                    // <num_motionblur_keys>
                                                    // <data_type>: VECTOR
                                                    // <elem1> <elem2>
                                                    // <elem3> <elem4>
                                                    // ...
                                                    let mut num_elements: u32 = 0;
                                                    let mut num_motionblur_keys: u32 = 1;
                                                    let data_type: String = String::from("VECTOR");
                                                    let mut elems: Vec<Float> = Vec::new();
                                                    if let Some(num_elements_str) = iter.next() {
                                                        num_elements =
                                                            u32::from_str(num_elements_str)
                                                                .unwrap();
                                                        if let Some(num_motionblur_keys_str) =
                                                            iter.next()
                                                        {
                                                            num_motionblur_keys =
                                                                u32::from_str(num_motionblur_keys_str).unwrap();
                                                            if let Some(data_type_str) = iter.next()
                                                            {
                                                                if data_type_str != data_type {
                                                                    panic!(
                                                                        "ERROR: {} expected ...",
                                                                        data_type
                                                                    );
                                                                } else {
                                                                    let expected: u32 = num_elements * num_motionblur_keys * 3;
                                                                    for _i in 0..expected {
                                                                        if let Some(elem_str) =
                                                                            iter.next()
                                                                        {
                                                                            let elem: f32 =
                                                                                f32::from_str(elem_str)
                                                                                .unwrap();
                                                                            elems.push(
                                                                                elem as Float,
                                                                            );
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                    print!(
                                                        "\n vlist {} {} VECTOR ... ",
                                                        num_elements, num_motionblur_keys
                                                    );
                                                    println!("\n {:?}", elems);
                                                    // TriangleMesh
                                                    let mut x: Float = 0.0;
                                                    let mut y: Float = 0.0;
                                                    let mut z;
                                                    let mut p: Vec<Point3f> = Vec::new();
                                                    for i in 0..elems.len() {
                                                        if i % 3 == 0 {
                                                            x = elems[i];
                                                        } else if i % 3 == 1 {
                                                            y = elems[i];
                                                        } else {
                                                            // i % 3 == 2
                                                            z = elems[i];
                                                            // store as Point3f
                                                            p.push(Point3f {
                                                                x: x,
                                                                y: y,
                                                                z: z,
                                                            });
                                                        }
                                                    }
                                                    for point in p {
                                                        println!(" {:?}", point);
                                                    }

                                                }
                                            }
                                        } else {
                                            println!("}}");
                                        }
                                    } else {
                                        break;
                                    }
                                }
                            }
                            // WORK
                            _ => println!("TODO: {:?}", inner_pair.as_rule()),
                        }
                    }
                }
            }
            None => panic!("No input file name."),
        }
        println!("render_camera = {:?} ", render_camera);
        println!("fov = {:?} ", fov);
        println!("filter_name = {:?}", filter_name);
        println!("filter_width = {:?}", filter_width);
        println!("max_depth = {:?}", max_depth);
        // MakeFilter
        let mut some_filter: Option<Arc<Filter + Sync + Send>> = None;
        if filter_name == String::from("box") {
            println!("TODO: CreateBoxFilter");
        } else if filter_name == String::from("gaussian") {
            let mut filter_params: ParamSet = ParamSet::default();
            filter_params.add_float(String::from("xwidth"), filter_width);
            filter_params.add_float(String::from("ywidth"), filter_width);
            some_filter = Some(GaussianFilter::create(&filter_params));
        } else if filter_name == String::from("mitchell") {
            println!("TODO: CreateMitchellFilter");
        } else if filter_name == String::from("sinc") {
            println!("TODO: CreateSincFilter");
        } else if filter_name == String::from("triangle") {
            println!("TODO: CreateTriangleFilter");
        } else {
            panic!("Filter \"{}\" unknown.", filter_name);
        }
        // MakeFilm
        let resolution: Point2i = Point2i { x: xres, y: yres };
        println!("resolution = {:?}", resolution);
        if let Some(filter) = some_filter {
            let crop: Bounds2f = Bounds2f {
                p_min: Point2f { x: 0.0, y: 0.0 },
                p_max: Point2f { x: 1.0, y: 1.0 },
            };
            let diagonal: Float = 35.0;
            let scale: Float = 1.0;
            let max_sample_luminance: Float = std::f32::INFINITY;
            let film: Arc<Film> = Arc::new(Film::new(
                resolution,
                crop,
                filter,
                diagonal,
                String::from(""),
                scale,
                max_sample_luminance,
            ));
            // MakeCamera
            let mut some_camera: Option<Arc<Camera + Sync + Send>> = None;
            let mut medium_interface: MediumInterface = MediumInterface::default();
            let camera_to_world: TransformSet = TransformSet {
                t: [Transform {
                    m: Matrix4x4 {
                        m: [
                            [1.0, 0.0, 0.0, 0.0],
                            [0.0, 1.0, 0.0, 0.0],
                            [0.0, 0.0, 1.0, 0.0],
                            [0.0, 0.0, 0.0, 1.0],
                        ],
                    },
                    m_inv: Matrix4x4 {
                        m: [
                            [1.0, 0.0, 0.0, 0.0],
                            [0.0, 1.0, 0.0, 0.0],
                            [0.0, 0.0, 1.0, 0.0],
                            [0.0, 0.0, 0.0, 1.0],
                        ],
                    },
                }; 2],
            };
            let transform_start_time: Float = 0.0;
            let transform_end_time: Float = 1.0;
            let animated_cam_to_world: AnimatedTransform = AnimatedTransform::new(
                &camera_to_world.t[0],
                transform_start_time,
                &camera_to_world.t[1],
                transform_end_time,
            );
            if camera_name == String::from("perspective") {
                let mut camera_params: ParamSet = ParamSet::default();
                camera_params.add_float(String::from("fov"), fov);
                let camera: Arc<Camera + Send + Sync> = PerspectiveCamera::create(
                    &camera_params,
                    animated_cam_to_world,
                    film,
                    medium_interface.outside,
                );
                some_camera = Some(camera);
            } else if camera_name == String::from("orthographic") {
                println!("TODO: CreateOrthographicCamera");
            } else if camera_name == String::from("realistic") {
                println!("TODO: CreateRealisticCamera");
            } else if camera_name == String::from("environment") {
                println!("TODO: CreateEnvironmentCamera");
            } else {
                panic!("Camera \"{}\" unknown.", camera_name);
            }
            if let Some(camera) = some_camera {
                // MakeSampler
                let mut some_sampler: Option<Box<Sampler + Sync + Send>> = None;
                // use SobolSampler for now
                let nsamp: i64 = 16; // TODO: something from .ass file
                let sample_bounds: Bounds2i = camera.get_film().get_sample_bounds();
                let sampler = Box::new(SobolSampler::new(nsamp, sample_bounds));
                some_sampler = Some(sampler);
                if let Some(mut sampler) = some_sampler {
                    // MakeIntegrator
                    let mut some_integrator: Option<
                        Box<SamplerIntegrator + Sync + Send>,
                    > = None;
                    // CreatePathIntegrator
                    let pixel_bounds: Bounds2i = camera.get_film().get_sample_bounds();
                    let rr_threshold: Float = 1.0;
                    let light_strategy: String = String::from("spatial");
                    let integrator = Box::new(PathIntegrator::new(
                        max_depth as u32,
                        pixel_bounds,
                        rr_threshold,
                        light_strategy,
                    ));
                    some_integrator = Some(integrator);
                    if let Some(mut integrator) = some_integrator {
                        // MakeIntegrator
                        if lights.is_empty() {
                            // warn if no light sources are defined
                            print!("WARNING: No light sources defined in scene; ");
                            println!("rendering a black image.");
                        }
                        if !primitives.is_empty() {
                            let split_method = SplitMethod::SAH;
                            let max_prims_in_node: i32 = 4;
                            let accelerator = Arc::new(BVHAccel::new(
                                primitives.clone(),
                                max_prims_in_node as usize,
                                split_method,
                            ));
                            let scene: Scene = Scene::new(accelerator.clone(), lights.clone());
                            let num_threads: u8 = num_cpus::get() as u8;
                            render(
                                &scene,
                                &camera.clone(),
                                &mut sampler,
                                &mut integrator,
                                num_threads,
                            );
                        } else {
                            print!("WARNING: No primitives defined in scene; ");
                            println!("no need to render anything.");
                        }
                    }
                }
            }
        }
        return;
    } else if matches.opt_present("v") {
        print_version(&program);
        return;
    } else {
        print_usage(&program, opts);
        return;
    }
}
