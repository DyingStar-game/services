import { decode, encode } from "@msgpack/msgpack";
import { ConnectedMessageWsType } from "@websocket/schema/Response/connected.ws.model";
import { ErrorMessageWsType } from "@websocket/schema/Response/error.ws.model";
import { ResponseInitWsType } from "@websocket/schema/Response/init.ws.model";
import { ResponseWsType } from "@websocket/schema/Response/response.ws.model";
import WebSocket from "ws";

export class TestWebSocketClient {
  private ws: WebSocket | null = null;
  private messages: ResponseWsType[] = [];
  private connected: boolean = false;

  constructor(private port: number = 3099) {}

  async connect(): Promise<void> {
    return new Promise((resolve, reject) => {
      this.ws = new WebSocket(`ws://localhost:${this.port}`);

      this.ws.on("open", () => {
        this.connected = true;
        resolve();
      });

      this.ws.on("message", (data: Buffer) => {
        const decoded = decode(data) as ResponseWsType;
        this.messages.push(decoded);
      });

      this.ws.on("error", (error) => {
        reject(error);
      });

      // Timeout after 5 secondes
      setTimeout(() => {
        if (!this.connected) {
          reject(new Error("Connection timeout"));
        }
      }, 5000);
    });
  }

  send(message: any): void {
    if (!this.ws || !this.connected) {
      throw new Error("WebSocket not connected");
    }

    const encoded = encode(message);
    this.ws.send(encoded);
  }

  async waitForMessage(timeout: number = 5000): Promise<ResponseWsType> {
    const startTime = Date.now();

    while (this.messages.length === 0) {
      if (Date.now() - startTime > timeout) {
        throw new Error("Timeout waiting for message");
      }
      await new Promise((resolve) => setTimeout(resolve, 10));
    }

    return this.messages.shift()!;
  }

  async waitForConnected(
    timeout: number = 5000,
  ): Promise<ConnectedMessageWsType> {
    const msg = await this.waitForMessage(timeout);

    if ("type" in msg && msg.type === "connected") {
      return msg;
    }

    throw new Error(`Expected connected message, got: ${JSON.stringify(msg)}`);
  }

  async waitForInit(timeout: number = 5000): Promise<ResponseInitWsType> {
    const msg = await this.waitForMessage(timeout);

    if ("event" in msg && msg.event === "create_object") {
      return msg;
    }

    throw new Error(`Expected init message, got: ${JSON.stringify(msg)}`);
  }

  async waitForError(timeout: number = 5000): Promise<ErrorMessageWsType> {
    const msg = await this.waitForMessage(timeout);

    if ("error" in msg) {
      return msg;
    }

    throw new Error(`Expected error message, got: ${JSON.stringify(msg)}`);
  }

  getMessages(): unknown[] {
    return [...this.messages];
  }

  clearMessages(): void {
    this.messages = [];
  }

  close(): void {
    if (this.ws) {
      this.ws.close();
      this.ws = null;
      this.connected = false;
    }
  }

  isConnected(): boolean {
    return this.connected;
  }
}
