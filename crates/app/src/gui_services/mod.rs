#[cfg(test)]
mod gutter_job_tests;
mod jobs;
#[cfg(test)]
mod log_job_tests;
mod submit;
#[cfg(test)]
mod submit_tests;

pub(crate) use jobs::COMMIT_SUBSYSTEM;
pub use jobs::ImageJobs;
#[cfg(test)]
pub(crate) use submit::project_directories;
pub use submit::{Owner, Services, Submit};

/// What a refused job answers its view (see `jobs::replies`).
#[cfg(test)]
pub(crate) fn rejection_for_test(job: &crate::Job) -> Vec<crate::Update> {
    jobs::JobRouting::rejection(job)
}
