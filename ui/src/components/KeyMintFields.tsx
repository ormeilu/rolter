import { useTranslation } from "react-i18next";

import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { useFormat } from "@/lib/i18n/format";

// the name + expiry + reach block shared by the two screens that mint a virtual
// key: the admin one (Keys) and the self-service one (Account). #945 made both
// of those choices required, and a rule enforced on one screen only is a rule
// with a way around it.

/** presets the expiry picker offers, in days; `null` is the deliberate "never" */
export const KEY_TTL_CHOICES: (number | null)[] = [7, 30, 60, 90, null];
/** what an operator gets if they change nothing — finite, by design */
export const DEFAULT_KEY_TTL_DAYS = 30;
/** mirrors `MAX_KEY_NAME_LEN` in `crates/rolter-control/src/me.rs` */
export const MAX_KEY_NAME_LEN = 64;
/** the select's "never" token: its own value, so an unset control can never be
 *  mistaken for a chosen "never" */
export const NEVER = "never";

/** `undefined` means "never expires", which is what the API wants omitted */
export function ttlToDays(ttl: string): number | undefined {
  return ttl === NEVER ? undefined : Number(ttl);
}

/** the same rule the control plane applies, checked here so the operator learns
 *  it before spending a round trip */
export function keyNameProblem(name: string): "blank" | "long" | null {
  const trimmed = name.trim();
  if (trimmed.length === 0) return "blank";
  if (trimmed.length > MAX_KEY_NAME_LEN) return "long";
  return null;
}

export function KeyNameField({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  const tooLong = keyNameProblem(value) === "long";
  return (
    <Field
      label={t("keyMint.name")}
      error={tooLong ? t("keyMint.nameTooLong", { max: MAX_KEY_NAME_LEN }) : undefined}
      hint={tooLong ? undefined : t("keyMint.nameHint")}
    >
      <Input
        required
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={t("keyMint.namePlaceholder")}
      />
    </Field>
  );
}

export function KeyExpiryField({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <Field
      label={t("keyMint.expiry")}
      hint={value === NEVER ? t("keyMint.expiryNeverWarning") : t("keyMint.expiryHint")}
    >
      <Select value={value} onChange={(e) => onChange(e.target.value)}>
        {KEY_TTL_CHOICES.map((days) => (
          <option key={days ?? NEVER} value={days === null ? NEVER : String(days)}>
            {days === null
              ? t("keyMint.expiryNever")
              : t("keyMint.expiryDays", { count: days })}
          </option>
        ))}
      </Select>
    </Field>
  );
}

export function KeyModelsField({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <Field label={t("keyMint.models")} hint={t("keyMint.modelsHint")}>
      <Input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={t("keyMint.modelsPlaceholder")}
      />
    </Field>
  );
}

/**
 * The per-key response-cache override.
 *
 * Three states, not a switch: "inherit" is the absence of a decision and is
 * what a key gets when nobody made one, while "off" and "on" both override the
 * route. Collapsing that to a boolean would turn "I did not choose" into "I
 * chose no".
 */
export function KeyCacheField({
  value,
  onChange,
}: {
  value: CacheMode;
  onChange: (value: CacheMode) => void;
}) {
  const { t } = useTranslation();
  return (
    <Field label={t("keyMint.cache")} hint={t("keyMint.cacheHint")}>
      <Select
        aria-label={t("keyMint.cache")}
        value={value}
        onChange={(e) => onChange(e.target.value as CacheMode)}
      >
        <option value="inherit">{t("keyMint.cacheInherit")}</option>
        <option value="off">{t("keyMint.cacheOff")}</option>
        <option value="on">{t("keyMint.cacheOn")}</option>
      </Select>
    </Field>
  );
}

/** the three states `cache` can be in on the wire: null, false, true */
export type CacheMode = "inherit" | "off" | "on";

export function cacheMode(cache: boolean | null | undefined): CacheMode {
  if (cache === true) return "on";
  if (cache === false) return "off";
  return "inherit";
}

/** `null` is the wire value for "inherit", which is not the same as `false` */
export function parseCacheMode(value: string): boolean | null {
  if (value === "on") return true;
  if (value === "off") return false;
  return null;
}

/** split a comma-separated allow-list into the array the API takes */
export function parseModels(text: string): string[] {
  return text
    .split(",")
    .map((m) => m.trim())
    .filter(Boolean);
}

/**
 * What the key will be able to reach, stated before it exists. The secret is
 * shown exactly once, so "did I just mint something narrow, or something that
 * can spend money against every provider?" has to be answerable now.
 */
export function KeyReachSummary({
  project,
  models,
  providers = [],
  ttl,
}: {
  project: string;
  models: string[];
  /** provider slugs the key is narrowed to; empty is every provider */
  providers?: string[];
  ttl: string;
}) {
  const { t } = useTranslation();
  const format = useFormat();
  const days = ttlToDays(ttl);
  const expiryDate = days === undefined ? null : new Date(Date.now() + days * 86_400_000);

  return (
    <div className="space-y-1 rounded-md border border-dashed border-border bg-muted/40 p-3">
      <p className="text-xs font-medium text-foreground">{t("keyMint.reach.title")}</p>
      <ul className="space-y-0.5 text-xs text-muted-foreground">
        <li>{t("keyMint.reach.project", { project })}</li>
        <li>
          {models.length === 0
            ? t("keyMint.reach.allModels")
            : t("keyMint.reach.someModels", {
                count: models.length,
                models: models.join(", "),
              })}
        </li>
        <li>
          {providers.length === 0
            ? t("keyMint.reach.allProviders")
            : t("keyMint.reach.someProviders", {
                count: providers.length,
                providers: providers.join(", "),
              })}
        </li>
        <li>
          {expiryDate === null
            ? t("keyMint.reach.never")
            : t("keyMint.reach.until", { date: format.date(expiryDate) })}
        </li>
      </ul>
    </div>
  );
}
