import { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import { theme } from "./theme";
import type { LlmMode, OnboardingState } from "./types";
import { AgentChannelStep } from "./wizard/AgentChannelStep";
import { AnswerModelStep } from "./wizard/AnswerModelStep";
import { AskStep } from "./wizard/AskStep";
import { ModeStep } from "./wizard/ModeStep";
import { SourceStep } from "./wizard/SourceStep";
import { styles } from "./wizard/styles";
import { stepLabel, stepsFor, type StepId } from "./wizard/wizardText";

/**
 * First-run wizard (R45.6). Mode first: who writes the answers decides the
 * rest — local mode picks a RAM-sized answer model, agent mode picks a
 * channel (MCP or the REST skill). Search embeddings come from the built-in
 * model; Ollama is needed only for local answers (or faster indexing).
 */
export function OnboardingWizard({ onClose }: { onClose: () => void }) {
  const [state, setState] = useState<OnboardingState | null>(null);
  const [mode, setModeState] = useState<LlmMode>("local");
  const [embedModel, setEmbedModel] = useState("nomic-embed-text");
  const [step, setStep] = useState<StepId>("setup");
  const [error, setError] = useState<string | null>(null);

  const steps = stepsFor(mode);
  const index = Math.max(0, steps.indexOf(step));
  const last = index === steps.length - 1;

  const recheck = useCallback(async (): Promise<OnboardingState | null> => {
    try {
      const fresh = await api.onboarding();
      setState(fresh);
      setError(null);
      return fresh;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return null;
    }
  }, []);

  useEffect(() => {
    api
      .onboarding()
      .then(setState)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
    api
      .getSettings()
      .then((s) => {
        setModeState(s.llm_mode === "agent" ? "agent" : "local");
        if (s.embed_model) setEmbedModel(s.embed_model);
      })
      .catch(() => {});
  }, []);

  // Readiness flags are mode-aware server-side, so switching persists
  // llm_mode and re-reads the checks.
  async function setMode(m: LlmMode) {
    setModeState(m);
    try {
      await api.updateSettings({ llm_mode: m });
      await recheck();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const go = (to: StepId) => {
    setStep(to);
    if (to === "ask" || to === "answers") recheck();
  };
  const next = () => (last ? onClose() : go(steps[index + 1]));
  const back = () => index > 0 && go(steps[index - 1]);
  const sourceMissing = step === "source" && (state?.sources_count ?? 0) === 0;

  return (
    <div style={styles.backdrop}>
      <div style={styles.card} role="dialog" aria-label="Welcome to AnchorCore">
        <div style={styles.header}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: 12 }}>
            <div>
              <h2 style={{ margin: 0, fontSize: 20 }}>Welcome to AnchorCore</h2>
              <div style={{ fontSize: 13, color: theme.textMuted, marginTop: 2 }}>
                Connect a folder, and AnchorCore turns it into searchable company memory.
              </div>
            </div>
            <button
              onClick={onClose}
              title="Skip the wizard — you can connect a source later in Sources"
              style={{ ...styles.button, ...styles.ghost, flexShrink: 0 }}
            >
              Skip →
            </button>
          </div>
        </div>

        <div style={styles.steps}>
          {steps.map((id, i) => (
            <span key={id} style={{ ...styles.step, ...(i === index ? styles.stepActive : {}), ...(i < index ? styles.stepDone : {}) }}>
              {i + 1}. {stepLabel(id, mode)}
            </span>
          ))}
        </div>

        {error && <p style={{ color: theme.red }}>{error}</p>}

        {step === "setup" && <ModeStep state={state} mode={mode} embedModel={embedModel} onMode={setMode} recheck={async () => void (await recheck())} />}
        {step === "answers" && <AnswerModelStep state={state} recheck={recheck} />}
        {step === "agent" && <AgentChannelStep mode={mode} />}
        {step === "source" && (
          <SourceStep
            state={state}
            onSynced={async () => {
              await recheck();
              go("ask");
            }}
          />
        )}
        {step === "ask" && <AskStep state={state} mode={mode} />}

        <div style={styles.actions}>
          {index > 0 ? (
            <button style={styles.button} onClick={back}>
              ← Back
            </button>
          ) : (
            <button style={{ ...styles.button, ...styles.ghost }} onClick={onClose}>
              Skip for now
            </button>
          )}
          {step === "answers" || (step === "agent" && mode === "local") ? (
            <button style={{ ...styles.button, ...styles.ghost }} onClick={next}>
              {last ? "Skip — open AnchorCore" : "Skip"}
            </button>
          ) : null}
          <button
            style={{ ...styles.button, ...styles.primary }}
            disabled={sourceMissing}
            title={sourceMissing ? "Connect a folder or the sample first" : undefined}
            onClick={next}
          >
            {last ? "Done — open AnchorCore" : "Continue →"}
          </button>
        </div>
      </div>
    </div>
  );
}
