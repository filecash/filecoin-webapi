use std::sync::{Arc, Mutex};
// use actix_web::FromRequest;
use actix_web::{error, middleware, web};
use actix_web::{App, HttpRequest, HttpResponse, HttpServer};
use log::*;
// use crate::seal_data::SealCommitPhase2Data;
use crate::config::Config;
use crate::mid::verify::Verify;
use crate::system::ServState;
use actix_web::middleware::Condition;
use clap::Arg;
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};
use std::fs::metadata;

mod config;
mod mid;
mod polling;
pub mod post;
pub mod post_data;
pub mod seal;
pub mod seal_data;
mod system;
mod types;

#[allow(dead_code)]
fn json_error_handler(err: error::JsonPayloadError, _req: &HttpRequest) -> error::Error {
    error!("{:?}", err);

    let detail = err.to_string();
    let response = match &err {
        error::JsonPayloadError::ContentType => HttpResponse::UnsupportedMediaType()
            .content_type("text/plain")
            .body(detail),
        _ => HttpResponse::BadRequest().content_type("text/plain").body(detail),
    };
    error::InternalError::from_response(err, response).into()
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    let m = clap::App::new("filecoin-webapi")
        .author("sbw <sbw@sbw.so>")
        .version("2.0.0")
        .about("filecoin webapi")
        .arg(
            Arg::with_name("config")
                .short("-c")
                .long("--config")
                .help("Config file location")
                .takes_value(true)
                .required(true)
                .default_value("/etc/filecoin-webapi.conf"),
        )
        .get_matches();

    // default logger settings
    if std::env::var("RUST_LOG").is_err() {
        if cfg!(debug_assertions) {
            std::env::set_var("RUST_LOG", "filecoin_webapi=trace,actix_web=debug");
        } else {
            std::env::set_var("RUST_LOG", "filecoin_webapi=debug,actix_web=info");
        }
    }

    env_logger::init();

    // if TMPDIR is set, ensure dir is exist
    if let Ok(tmp) = std::env::var("TMPDIR") {
        match metadata(&tmp).map(|x| x.is_dir()) {
            Ok(true) => {}
            _ => panic!("TMPDIR={} not a valid directory!", tmp),
        }
    }

    // if CUDA_VISIBLE_DEVICES, print to warning log
    if let Ok(cuda_devs) = std::env::var("CUDA_VISIBLE_DEVICES") {
        warn!("CUDA_VISIBLE_DEVICES={}", cuda_devs);
    }

    // create upload dir
    std::fs::create_dir_all("/tmp/upload/")?;

    let config_file = m.value_of("config").unwrap();
    let f = std::fs::File::open(config_file).unwrap();
    let config: Config = serde_yaml::from_reader(f).unwrap();
    info!("config {:?}", config);

    let state = Arc::new(Mutex::new(ServState::new(config.clone())));
    let bind_addr = config.listen_addr.clone();
    let auth = config.auth;
    let private_cert = config.private_cert.clone();
    let cert_chain = config.cert_chain.clone();
    let server = HttpServer::new(move || {
        let state = state.clone();

        App::new()
            .wrap(middleware::Logger::default())
            .wrap(Condition::new(auth, Verify {}))
            .app_data(web::Data::new(state))
            .service(web::resource("/test").route(web::get().to(system::test)))
            .service(web::resource("/sys/test_polling").route(web::post().to(system::test_polling)))
            .service(web::resource("/sys/query_state").route(web::post().to(system::query_state)))
            .service(web::resource("/sys/debug_info").route(web::post().to(system::debug_info)))
            .service(web::resource("/sys/remove_job").route(web::post().to(system::remove_job)))
            .service(web::resource("/sys/upload_file").route(web::post().to(system::upload_file)))
            .service(web::resource("/sys/upload_test").route(web::get().to(system::upload_test)))
            .service(web::resource("/sys/get_worker_info").route(web::get().to(system::get_worker_info)))
            .service(
                web::resource("/post/generate_winning_post_sector_challenge")
                    .route(web::post().to(post::generate_winning_post_sector_challenge)),
            )
            .service(web::resource("/post/generate_winning_post").route(web::post().to(post::generate_winning_post)))
            .service(web::resource("/post/verify_winning_post").route(web::post().to(post::verify_winning_post)))
            .service(web::resource("/post/generate_window_post").route(web::post().to(post::generate_window_post)))
            .service(web::resource("/post/verify_window_post").route(web::post().to(post::verify_window_post)))
            .service(web::resource("/seal/clear_cache").route(web::post().to(seal::clear_cache)))
            .service(web::resource("/seal/seal_pre_commit_phase1").route(web::post().to(seal::seal_pre_commit_phase1)))
            .service(web::resource("/seal/seal_pre_commit_phase2").route(web::post().to(seal::seal_pre_commit_phase2)))
            .service(web::resource("/seal/compute_comm_d").route(web::post().to(seal::compute_comm_d)))
            .service(web::resource("/seal/seal_commit_phase1").route(web::post().to(seal::seal_commit_phase1)))
            .service(
                web::resource("/seal/seal_commit_phase2")
                    // .app_data(web::Json::<SealCommitPhase2Data>::configure(|cfg| {
                    // cfg.limit(1024000)
                    //         .content_type(|_mime| true)
                    //         .error_handler(json_error_handler)
                    // }))
                    .route(web::post().to(seal::seal_commit_phase2)),
            )
            .service(web::resource("/seal/verify_seal").route(web::post().to(seal::verify_seal)))
            .service(web::resource("/seal/verify_batch_seal").route(web::post().to(seal::verify_batch_seal)))
            .service(web::resource("/seal/get_unsealed_range").route(web::post().to(seal::get_unsealed_range)))
            .service(
                web::resource("/seal/generate_piece_commitment").route(web::post().to(seal::generate_piece_commitment)),
            )
            .service(web::resource("/seal/add_piece").route(web::post().to(seal::add_piece)))
            .service(web::resource("/seal/write_and_preprocess").route(web::post().to(seal::write_and_preprocess)))
    });

    let server = if let Some(cert) = private_cert {
        warn!("use private-cert file {}", cert);

        let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
        builder.set_private_key_file(cert, SslFiletype::PEM).unwrap();
        if let Some(chain) = cert_chain {
            warn!("use cert-chain file {}", chain);
            builder.set_certificate_chain_file(chain).unwrap();
        }

        server.bind_openssl(bind_addr, builder)
    } else {
        server.bind(bind_addr)
    };

    server?.workers(1).run().await
}
