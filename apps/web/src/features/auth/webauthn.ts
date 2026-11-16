import type { components } from "@knoted/api-client";

type CredentialJson = components["schemas"]["WebAuthnCredential"];
function decode(value: string): ArrayBuffer {
  const binary = atob(value.replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(binary, (char) => char.charCodeAt(0)).buffer;
}
function encode(value: ArrayBuffer): string {
  return btoa(String.fromCharCode(...new Uint8Array(value)))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}
/** webauthn-rs sends a publicKey wrapper containing base64url binary fields. */
export async function runWebAuthn(
  options: Record<string, unknown>,
  kind: "create" | "get",
  signal?: AbortSignal,
): Promise<CredentialJson> {
  if (
    !navigator.credentials ||
    typeof navigator.credentials[kind] !== "function"
  )
    throw new Error("当前浏览器不支持 Passkey，请使用支持 WebAuthn 的浏览器。");
  const publicKey = structuredClone(options.publicKey) as Record<
    string,
    unknown
  >;
  if (!publicKey || typeof publicKey.challenge !== "string")
    throw new Error("认证参数无效，请重试。");
  publicKey.challenge = decode(publicKey.challenge);
  if (kind === "create") {
    const user = publicKey.user as Record<string, unknown>;
    publicKey.user = { ...user, id: decode(user.id as string) };
  }
  for (const key of ["allowCredentials", "excludeCredentials"]) {
    if (Array.isArray(publicKey[key])) {
      publicKey[key] = (publicKey[key] as Array<Record<string, unknown>>).map(
        (entry) => ({ ...entry, id: decode(entry.id as string) }),
      );
    }
  }
  const credential = (
    kind === "create"
      ? await navigator.credentials.create({
          publicKey: publicKey as unknown as PublicKeyCredentialCreationOptions,
          signal,
        })
      : await navigator.credentials.get({
          publicKey: publicKey as unknown as PublicKeyCredentialRequestOptions,
          signal,
        })
  ) as PublicKeyCredential | null;
  signal?.throwIfAborted();
  if (!credential) throw new Error("认证已取消，请重试。");
  const response = credential.response;
  const data: Record<string, unknown> = {
    clientDataJSON: encode(response.clientDataJSON),
  };
  if (kind === "create") {
    const registration = response as AuthenticatorAttestationResponse;
    data.attestationObject = encode(registration.attestationObject);
    data.transports = registration.getTransports?.() ?? [];
  } else {
    const assertion = response as AuthenticatorAssertionResponse;
    data.authenticatorData = encode(assertion.authenticatorData);
    data.signature = encode(assertion.signature);
    data.userHandle = assertion.userHandle
      ? encode(assertion.userHandle)
      : null;
  }
  return {
    id: credential.id,
    rawId: encode(credential.rawId),
    type: "public-key",
    response: data,
  };
}
