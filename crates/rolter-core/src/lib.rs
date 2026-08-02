//! Shared building blocks for the rolter gateway and control plane.
//!
//! This crate holds the configuration model, domain error type and telemetry
//! bootstrap that every other rolter crate depends on.

pub mod config;
pub mod currency;
pub mod error;
pub mod guardrail_webhook;
pub mod guardrails;
pub mod prompt_templates;
pub mod slug;
pub mod telemetry;

pub use config::{
    AdaptiveRoutingConfig, AdvancedModelConfig, ApiKeyConfig, BackpressurePolicy,
    BalancingStrategy, BreakerConfig, BudgetConfig, BudgetPeriod, BudgetScope, CacheConfig,
    CompatibilityConfig, CooldownConfig, EgressPolicy, FeatureFlagsConfig, GatewayConfig,
    GroupMember, HealthConfig, KvEventsConfig, LmCacheConfig, LoggingConfig, McpOAuthSessionConfig,
    McpServerConfig, MetricsScrapeConfig, ModelLimits, ModelPriceConfig, ModelRoute,
    ModelUsagePricing, ModelVisibility, OverrideMode, ParamPolicy, PayloadCaptureConfig,
    ProviderConfig, ProviderGroupConfig, ProviderKind, QueueConfig, RateLimitConfig,
    RealtimeConfig, ResponsesConfig, RetryConfig, RoleProfile, RouteCache, SemanticCacheConfig,
    ServerConfig, Target, TimeoutConfig, TlsConfig, Variant, VirtualKeyConfig, VirtualKeyRecord,
    MAX_EXPLORATION_RATIO, RESERVED_PATHS,
};
pub use currency::{CurrencyConfig, CurrencyConverter, StaticRates, DEFAULT_BASE_CURRENCY};
pub use error::{Error, Result};
pub use guardrail_webhook::{
    FailureMode, GuardrailWebhookConfig, WebhookAuth, WebhookDecision, WebhookRequest,
    WebhookStage, WebhookTenant,
};
pub use guardrails::{
    BuiltinRule, CompiledGuardrails, GuardAction, GuardStage, GuardrailReport, GuardrailRule,
    GuardrailsConfig, ScanOutcome, StreamingPostCall,
};
pub use prompt_templates::{
    CompiledTemplates, Decorator, DecoratorPosition, DecoratorRole, PromptTemplate,
    PromptTemplateActivationScope, PromptTemplateRequestScope, PromptTemplatesConfig, RenderError,
    RenderedMessage, TemplateReport, TemplateVariable, TEMPLATE_VARS_FIELD,
};

/// Redis pub/sub channel the control plane publishes config-version bumps
/// on; gateways subscribe to it to trigger an immediate snapshot poll.
pub const CONFIG_CHANNEL: &str = "rolter.config";
