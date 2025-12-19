use std::{env, error::Error as StdError, io::BufRead, sync::Arc, time::Duration};

use anyhow::Context;
use chrono::Utc;
use der::EncodePem;
use futures::StreamExt;
use k8s_openapi::{
    api::certificates::v1alpha1::PodCertificateRequest,
    apimachinery::pkg::apis::meta::v1::{Condition, Time},
    serde_json::json,
};
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
    runtime::{Config, Controller, controller::Action, watcher},
};
use tracing::{debug, info, warn};
use vaultrs::client::{VaultClient, VaultClientSettingsBuilder};

use crate::{controller::Data, error::Error, intermediate_ca::IntermediateCA};

mod ca_certificate;
mod controller;
mod error;
mod intermediate_ca;
mod utils;

fn error_policy(object: Arc<PodCertificateRequest>, error: &Error, _ctx: Arc<Data>) -> Action {
    let pcr_name = object.metadata.name.as_deref().unwrap_or("unknown");
    warn!("Reconciliation error for PCR {}: {}", pcr_name, error);

    Action::requeue(Duration::from_secs(5))
}

// code mostly taken from the kube.rs example
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let client = Client::try_default().await?;

    let pcrs = Api::<PodCertificateRequest>::all(client.clone());

    info!("starting openbao-pki-controller");
    info!("press <enter> to force a reconciliation of all objects");

    let (mut reload_tx, reload_rx) = futures::channel::mpsc::channel(0);
    // Using a regular background thread since tokio::io::stdin() doesn't allow aborting reads,
    // and its worker prevents the Tokio runtime from shutting down.
    std::thread::spawn(move || {
        for _ in std::io::BufReader::new(std::io::stdin()).lines() {
            let _ = reload_tx.try_send(());
        }
    });

    let mut settings = VaultClientSettingsBuilder::default();

    settings.address(env::var("BAO_ADDR").context("Please set BAO_ADDR")?);

    if let Ok(token) = env::var("BAO_TOKEN") {
        settings.token(token);
    } else {
        // TODO: implement kubernetes authentication
    }

    let settings = settings.build()?;
    let bao = VaultClient::new(settings)?;

    let hostname = "";
    let namespace = "";
    let cluster_domain = "";
    let _common_name = format!("{}.{}.svc.{}", hostname, namespace, cluster_domain);

    // limit the controller to running a maximum of two concurrent reconciliations
    let config = Config::default().concurrency(2);

    let ca = IntermediateCA::new(bao);

    Controller::new(pcrs, watcher::Config::default())
        .with_config(config)
        .reconcile_all_on(reload_rx.map(|_| ()))
        .shutdown_on_signal()
        .run(
            controller::reconcile,
            error_policy,
            Arc::new(Data::new(client, ca)),
        )
        .for_each(|res| async move {
            match res {
                Ok(o) => info!("reconciled {:?}", o),
                Err(e) => warn!("reconcile failed: {}", e),
            }
        })
        .await;
    info!("controller terminated");
    Ok(())
}
