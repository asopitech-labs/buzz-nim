import { listen } from "@tauri-apps/api/event";
import {
  Check,
  KeyRound,
  LoaderCircle,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { cancelPairing, confirmPairingSas } from "@/shared/api/tauri";
import { joinIdentityRecoveryPairing } from "@/shared/api/tauriPairing";
import { Button } from "@/shared/ui/button";
import { Textarea } from "@/shared/ui/textarea";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

type TransferStep =
  | "idle"
  | "connecting"
  | "sas"
  | "sending"
  | "done"
  | "error";

function pairingErrorMessage(error: unknown) {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : "";
  return message || "We couldn't start identity transfer. Try another code.";
}

export function IdentityTransferCard({
  currentPubkey,
}: {
  currentPubkey?: string;
}) {
  const [step, setStep] = useState<TransferStep>("idle");
  const [pairingUri, setPairingUri] = useState("");
  const [sasCode, setSasCode] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const activeRef = useRef(false);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    listen<{ sas: string }>("pairing-sas-received", ({ payload }) => {
      if (!disposed && activeRef.current) {
        setSasCode(payload.sas);
        setStep("sas");
      }
    }).then((unlisten) => (disposed ? unlisten() : unlisteners.push(unlisten)));
    listen("pairing-complete", () => {
      if (!disposed && activeRef.current) {
        activeRef.current = false;
        setStep("done");
      }
    }).then((unlisten) => (disposed ? unlisten() : unlisteners.push(unlisten)));
    listen<{ message: string }>("pairing-error", ({ payload }) => {
      if (!disposed && activeRef.current) {
        activeRef.current = false;
        setError(pairingErrorMessage(payload.message));
        setStep("error");
      }
    }).then((unlisten) => (disposed ? unlisten() : unlisteners.push(unlisten)));
    listen<{ reason: string }>("pairing-aborted", ({ payload }) => {
      if (!disposed && activeRef.current) {
        activeRef.current = false;
        setError(`Identity transfer stopped: ${payload.reason}`);
        setStep("error");
      }
    }).then((unlisten) => (disposed ? unlisten() : unlisteners.push(unlisten)));

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
      if (activeRef.current) {
        activeRef.current = false;
        void cancelPairing();
      }
    };
  }, []);

  async function connect() {
    if (!currentPubkey || !pairingUri.trim()) return;
    activeRef.current = true;
    setStep("connecting");
    setError(null);
    setSasCode(null);
    try {
      await joinIdentityRecoveryPairing(pairingUri.trim());
      setPairingUri("");
    } catch (cause) {
      activeRef.current = false;
      setError(pairingErrorMessage(cause));
      setStep("error");
    }
  }

  async function confirm() {
    setStep("sending");
    try {
      await confirmPairingSas();
    } catch (cause) {
      activeRef.current = false;
      setError(pairingErrorMessage(cause));
      setStep("error");
    }
  }

  async function deny() {
    activeRef.current = false;
    await cancelPairing().catch(() => {});
    setError("The codes didn't match. Identity transfer was canceled.");
    setStep("error");
  }

  function reset() {
    activeRef.current = false;
    setStep("idle");
    setSasCode(null);
    setError(null);
  }

  return (
    <section className="min-w-0" data-testid="settings-identity-transfer">
      <SettingsSectionHeader
        title="Identity transfer"
        description="Send this signed-in Desktop identity only to a new Desktop you control. The receiving Desktop will be able to sign as you."
      />
      <SettingsOptionGroup
        className="w-full"
        data-testid="identity-transfer-card"
        surface="soft"
      >
        <p aria-live="polite" className="sr-only">
          {step === "sas" && sasCode
            ? `Verification code ${sasCode.slice(0, 3)} ${sasCode.slice(3)} ready.`
            : step === "sending"
              ? "Sending identity to the receiving Desktop."
              : step === "done"
                ? "Identity transferred."
                : ""}
        </p>
        <SettingsOptionRow className="flex-col items-stretch gap-5 p-6">
          {step === "idle" ? (
            <>
              <div className="space-y-1">
                <p className="text-base font-medium">Recovery code</p>
                <p
                  className="text-sm text-muted-foreground/70"
                  data-settings-subcopy
                >
                  On the new Desktop, choose Recover from another Desktop and
                  copy its one-time code here.
                </p>
              </div>
              <Textarea
                aria-label="Recovery code from the new Desktop"
                data-testid="identity-transfer-code"
                disabled={!currentPubkey}
                onChange={(event) => setPairingUri(event.target.value)}
                placeholder="nostrpair://…&mode=recover"
                rows={4}
                value={pairingUri}
              />
              <Button
                className="self-start"
                data-testid="start-identity-transfer"
                disabled={!currentPubkey || !pairingUri.trim()}
                onClick={() => void connect()}
                type="button"
              >
                <KeyRound className="mr-1.5 h-4 w-4" />
                Connect to receiving Desktop
              </Button>
            </>
          ) : step === "sas" && sasCode ? (
            <div className="flex max-w-md flex-col items-start gap-4">
              <ShieldCheck className="h-10 w-10 text-primary" />
              <div>
                <p className="text-base font-medium">
                  Does this code match the receiving Desktop?
                </p>
                <p
                  className="mt-1 text-sm text-muted-foreground/70"
                  data-settings-subcopy
                >
                  Confirm only if you started recovery on a new Desktop you
                  control.
                </p>
              </div>
              <p
                className="rounded-xl border-2 border-primary/30 bg-primary/5 px-5 py-3 font-mono text-3xl font-bold tracking-[0.25em]"
                data-testid="identity-transfer-sas"
              >
                {sasCode.slice(0, 3)} {sasCode.slice(3)}
              </p>
              <div className="flex gap-2">
                <Button
                  data-testid="confirm-identity-transfer"
                  onClick={() => void confirm()}
                >
                  <Check className="mr-1.5 h-4 w-4" />
                  Codes match — send identity
                </Button>
                <Button
                  data-testid="cancel-identity-transfer"
                  onClick={() => void deny()}
                  variant="outline"
                >
                  Cancel
                </Button>
              </div>
            </div>
          ) : step === "done" ? (
            <div className="flex items-center gap-3">
              <Check className="h-6 w-6 text-green-600 dark:text-green-400" />
              <div>
                <p className="text-base font-medium">Identity transferred</p>
                <p
                  className="text-sm text-muted-foreground/70"
                  data-settings-subcopy
                >
                  The receiving Desktop can now sign with this identity.
                </p>
              </div>
            </div>
          ) : step === "error" ? (
            <div className="flex items-start gap-3">
              <TriangleAlert className="mt-0.5 h-5 w-5 text-destructive" />
              <div className="space-y-3">
                <p className="text-sm text-destructive">{error}</p>
                <Button onClick={reset} size="sm" variant="outline">
                  Try another code
                </Button>
              </div>
            </div>
          ) : (
            <div
              className="flex items-center gap-3"
              data-testid="identity-transfer-progress"
            >
              <LoaderCircle className="h-5 w-5 animate-spin text-muted-foreground" />
              <p className="text-sm text-muted-foreground">
                {step === "sending"
                  ? "Sending identity to the receiving Desktop…"
                  : "Connecting to the receiving Desktop…"}
              </p>
            </div>
          )}
        </SettingsOptionRow>
      </SettingsOptionGroup>
    </section>
  );
}
