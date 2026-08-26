import { invokeTauri } from "@/shared/api/tauri";

export async function startIdentityRecoveryPairing(): Promise<string> {
  return invokeTauri<string>("start_identity_recovery_pairing");
}

export async function joinIdentityRecoveryPairing(
  pairingUri: string,
): Promise<void> {
  await invokeTauri("join_identity_recovery_pairing", { pairingUri });
}
