/**
 * TypeScript-Spiegel von messages.schema.json (Overlay + PWA).
 * Änderungen immer in beiden Dateien und in pc-app/core/src/protocol.rs nachziehen.
 */
export type Mood = "success" | "game" | "confused" | "error" | "info";
export type AvatarState = "idle" | "alert" | "listening" | "thinking" | "sleeping";
export type MicState = "off" | "wake" | "recording" | "muted";

export type ClientMessage =
  | { type: "command"; id: string; text: string }
  | { type: "invoke"; id: string; action: string; params: Record<string, unknown> }
  | { type: "confirm"; confirm_id: string; accept: boolean };

export interface Reply {
  type: "reply";
  id: string;
  ok: boolean;
  text: string;
  mood: Mood;
  action?: string | null;
}

export interface ConfirmRequest {
  type: "confirm_request";
  id: string;
  confirm_id: string;
  text: string;
  tier: 1 | 2 | 3; // 1 = Rückfrage "Meinst du …?"
  expires_in_ms: number;
  pc_only: boolean;
}

export interface State {
  type: "state";
  avatar: AvatarState;
  mic: MicState;
  enabled: boolean;
  level?: number;
}

export type ServerMessage = Reply | ConfirmRequest | State;
