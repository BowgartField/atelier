pub mod auth;
pub mod capabilities;
mod capabilities_generated;
pub mod chat;
pub mod config;
pub mod contexts;
pub mod error;
pub mod events;
pub mod git;
pub mod git_status;
pub mod github;
pub mod linear;
pub mod names;
pub mod paths;
pub mod persistence;
pub mod projects;
pub mod runtime;
pub mod server;
pub mod sessions;
pub mod terminal;

pub use capabilities::{CapabilityClass, CommandCapability, HEADLESS_CAPABILITIES};
pub use chat::{ChatRunManager, ChatService};
pub use config::{
    read_jean_config, JeanConfig, JeanScripts, PortEntry, RunScript, ScriptRunner, ScriptService,
};
pub use contexts::{
    format_advisory_context_markdown, format_issue_context_markdown,
    format_linear_context_markdown, format_pr_context_markdown, format_security_context_markdown,
    generate_branch_name_from_advisory, generate_branch_name_from_issue,
    generate_branch_name_from_linear_issue, generate_branch_name_from_pr,
    generate_branch_name_from_security_alert, slugify_issue_title, AdvisoryContext,
    AdvisoryVulnerability, ContextRef, ContextReferences, ContextService, GitHubAuthor,
    GitHubComment, GitHubReview, IssueContext, LinearComment, LinearIssueContext, LinearUser,
    PrDiffLoader, PullRequestContext, SecurityAlertContext, WorktreeContexts,
};
pub use error::{BackendError, BackendErrorCode};
pub use events::{EventSink, ServerEventSink, WsBroadcaster, WsEvent};
pub use git::{GitPushResponse, GitRunner, GitService};
pub use git_status::{ActiveWorktreeInfo, GitBranchStatus};
pub use github::{
    GhRunner, GitHubIssue, GitHubIssueDetail, GitHubIssueListResult, GitHubLabel,
    GitHubPullRequest, GitHubPullRequestDetail, GitHubService,
};
pub use linear::{
    LinearConfig, LinearIssue, LinearIssueDetail, LinearIssueListResult, LinearIssueState,
    LinearLabel, LinearService, LinearTeam, LinearTransport, ReqwestLinearTransport,
};
pub use paths::{AppPaths, HeadlessAppPaths, ResolvedAppPaths};
pub use persistence::{PersistenceService, ProjectsSnapshot};
pub use projects::{
    BaseSessionCloseMode, CheckoutPrPreparation, ExistingBranchCreationTask,
    ExistingBranchWorktreeInput, PrCheckout, ProjectService, WorktreeCreationInput,
    WorktreeCreationTask,
};
pub use runtime::{BackendContext, BackendState, ResourceRegistry};
pub use sessions::SessionService;
pub use terminal::TerminalManager;
