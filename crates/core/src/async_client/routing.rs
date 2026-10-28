//! Request routing: archived owners, redirects, and owner fan-out.
use super::*;
use futures_util::{StreamExt, stream};

/// One fan-out destination: the owner's paths, the request it receives, and
/// whether its failure fails the whole call.
type Target = (Paths, Request, bool);

impl Client {
    /// Heartbeats and whole-history clears go to every live owner, and
    /// history clears also to retired owners through the active one.
    pub(super) async fn broadcast(&self, request: Request) -> Result<Response> {
        let targets = self.broadcast_targets(request).await?;
        let retired = targets
            .iter()
            .filter_map(|(_, request, _)| archived_mutation(request))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut responses = stream::iter(targets.into_iter().map(
            |(paths, request, required)| async move {
                let result = match self.direct(paths, request, None).await {
                    Ok(response) => response.checked().map_err(AsyncClientError::from),
                    Err(error) => Err(error),
                };
                (required, result)
            },
        ))
        .buffer_unordered(4);
        while let Some((required, result)) = responses.next().await {
            // Snapshot retains unavailable-owner diagnostics. Heartbeats
            // tolerate only failures of non-active owners; real mutations
            // and active-owner failures must remain visible to callers.
            if required {
                result?;
            }
        }
        for generation in retired {
            self.evict_historical(&generation);
        }
        Ok(Response::Ok)
    }

    async fn broadcast_targets(&self, original: Request) -> Result<Vec<Target>> {
        let paths = self.paths.clone();
        self.catalog
            .run(&CancellationToken::new(), move || {
                let catalog = generations::Catalog::open(&paths)?;
                let active = catalog.active()?;
                let mut targets = Vec::new();
                for owner in catalog.generations()? {
                    if owner.status == generations::Status::Prepared {
                        continue;
                    }
                    if !generations::historical(&owner, active.as_deref()) {
                        let required = !matches!(original, Request::Heartbeat { .. })
                            || active.as_deref() == Some(owner.id.as_str());
                        targets.push((owner.paths(), original.clone(), required));
                    } else if matches!(original, Request::ClearHistory { .. }) {
                        targets.push((
                            generations::owner_for(&paths, &Request::Snapshot)?,
                            Request::Archived {
                                generation: owner.id,
                                request: Box::new(original.clone()),
                            },
                            true,
                        ));
                    }
                }
                Ok(targets)
            })
            .await
    }

    /// Wraps a request for a retired owner's session in `Request::Archived`.
    pub(super) async fn route_archived(&self, request: Request) -> Result<Request> {
        let paths = self.paths.clone();
        self.catalog
            .run(&CancellationToken::new(), move || {
                if let Some(owner) = archived_generation(&paths, &request)?
                    && !matches!(
                        request,
                        Request::Archived { .. } | Request::Shutdown | Request::ShutdownIfIdle
                    )
                {
                    return Ok(Request::Archived {
                        generation: owner,
                        request: Box::new(request),
                    });
                }
                Ok(request)
            })
            .await
    }

    pub(super) async fn owner(&self, request: &Request) -> Result<Paths> {
        let paths = self.paths.clone();
        let route_request = request.clone();
        self.catalog
            .run(&CancellationToken::new(), move || {
                Ok(generations::owner_for(&paths, &route_request)?)
            })
            .await
    }

    /// A redirect may only name an owner this catalog registered.
    pub(super) async fn verify_redirect(&self, generation: String) -> Result<()> {
        let paths = self.paths.clone();
        self.catalog
            .run(&CancellationToken::new(), move || {
                if !generations::Catalog::open(&paths)?
                    .generations()?
                    .iter()
                    .any(|g| g.id == generation)
                {
                    return Err(IpcError::UnregisteredRedirect { generation }.into());
                }
                Ok(())
            })
            .await
    }
}
