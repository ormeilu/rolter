#!/usr/bin/env bash
# Print everything needed to drive the dogfooding stack by hand (#924).
#
# Every credential here is local-only by construction: the fleet keys
# authenticate against a script on loopback, and the service passwords are the
# ones `docker/docker-compose.yml` hardcodes for the local stack. Nothing in
# this file is a secret, which is why it can be printed and checked in.
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KEK_FILE="${ROLTER_DOGFOOD_KEK_FILE:-$DIR/.kek}"

bold() { printf '\033[1m%s\033[0m\n' "$1"; }
dim() { printf '\033[2m%s\033[0m\n' "$1"; }
rule() { printf '\033[2m%s\033[0m\n' "────────────────────────────────────────────────────────────────────────"; }

# a service is "up" if something answers; keeps the sheet honest rather than
# printing a URL that is not listening
probe() {
  if curl -fsS -o /dev/null --max-time 2 "$1" 2>/dev/null; then printf '\033[32mup\033[0m'; else printf '\033[31mdown\033[0m'; fi
}

echo
bold "rolter · fuck-around mode"
dim "everything below is local-only. nothing here is a real credential."
echo

rule
bold "URLs"
rule
printf '  %-34s %-28s %s\n' "https://rolter.localhost" "dashboard" "$(probe http://127.0.0.1:4001/)"
printf '  %-34s %-28s %s\n' "https://api.rolter.localhost" "gateway /v1/*" "$(probe http://127.0.0.1:4000/metrics)"
printf '  %-34s %-28s %s\n' "https://signoz.localhost" "traces · metrics · logs" "$(probe http://127.0.0.1:8080/)"
dim "  (portless aliases; the raw ports are 4001 / 4000 / 8080)"
echo

rule
bold "Logins"
rule
printf '  %-22s %-30s %s\n' "dashboard" "admin@rolter.local" "dogfood-admin-2026"
printf '  %-22s %-30s %s\n' "signoz" "(sign up on first visit)" "kept local, no preset"
echo

rule
bold "Backing services"
rule
printf '  %-12s %-42s %-14s %s\n' "SERVICE" "URL" "USER" "PASSWORD"
printf '  %-12s %-42s %-14s %s\n' "postgres" "postgres://127.0.0.1:5432/rolter" "rolter" "rolter"
printf '  %-12s %-42s %-14s %s\n' "redis" "redis://127.0.0.1:6379" "-" "(no auth)"
printf '  %-12s %-42s %-14s %s\n' "clickhouse" "http://127.0.0.1:8123 (db: default)" "default" "(no password)"
printf '  %-12s %-42s %-14s %s\n' "otlp" "127.0.0.1:4317 grpc · 4318 http" "-" "(no auth)"
echo

rule
bold "Gateway auth"
rule
if [ -f "$DIR/.virtual-key" ]; then
  printf '  %-14s %s\n' "virtual key" "$(cat "$DIR/.virtual-key")"
else
  dim "  no virtual key yet — create one in the dashboard under Keys,"
  dim "  or: just dogfood-key"
fi
dim "  a brand-new key 401s for up to 5s until the gateway polls (#933)"
if [ -f "$KEK_FILE" ]; then
  printf '  %-14s %s\n' "ROLTER_KEK" "$(cat "$KEK_FILE")"
  dim "  (both planes must share this, or stored provider keys will not decrypt)"
else
  dim "  ROLTER_KEK not generated yet — 'just dogfood' makes one"
fi
echo

rule
bold "The fake fleet · add these as providers yourself"
rule
printf '  %-28s %-42s %s\n' "ENDPOINT" "MODEL_NAME" "API_KEY"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18001" "gpt-4o, gpt-4o-mini, gpt-4.1," "sk-dogfood-openai-4f9c2a7b1e"
printf '  %-28s %-42s %s\n' "" "gpt-4.1-mini, o3-mini" ""
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18002" "anthropic/claude-sonnet-4," "sk-or-v1-dogfood-83bd41e6c0"
printf '  %-28s %-42s %s\n' "" "meta-llama/llama-3.3-70b-instruct," ""
printf '  %-28s %-42s %s\n' "" "google/gemini-2.0-flash-001," ""
printf '  %-28s %-42s %s\n' "" "deepseek/deepseek-r1," ""
printf '  %-28s %-42s %s\n' "" "qwen/qwen-2.5-72b-instruct" ""
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18003" "meta-llama/Llama-3.1-8B-Instruct" "vllm-local-7c1d9e"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18004" "meta-llama/Llama-3.1-8B-Instruct" "-"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18005" "meta-llama/Llama-3.1-8B-Instruct" "-   (4x slower)"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18006" "Qwen/Qwen2.5-32B-Instruct" "vllm-local-2a8f43"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18007" "Qwen/Qwen2.5-32B-Instruct" "-"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18008" "mistralai/Mistral-7B-Instruct-v0.3" "vllm-local-b60c15"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18009" "mistralai/Mistral-7B-Instruct-v0.3" "-"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18010" "google/gemma-2-27b-it" "vllm-local-d4e701"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18011" "google/gemma-2-27b-it" "-"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18012" "deepseek-ai/DeepSeek-R1-Distill-Qwen-32B" "-   (503s 25% of the time)"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18013" "deepseek-ai/DeepSeek-R1-Distill-Qwen-32B" "vllm-local-9f22ac  (1.4s TTFT)"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18014" "BAAI/bge-large-en-v1.5" "-   (embeddings)"
printf '  %-28s %-42s %s\n' "http://127.0.0.1:18015" "intfloat/e5-mistral-7b-instruct" "vllm-local-3ba8d7  (embeddings)"
echo
dim "  kind: :18001 = openai · :18002 = openai_compatible (see #925) ·"
dim "        :18003-18013 = openai_compatible · :18014-18015 = tei"
dim "  the three marked targets exist so the health, breaker and latency"
dim "  screens have something other than green to show"
echo
