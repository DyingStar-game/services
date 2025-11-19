import { getInit, getUpdateObject } from "@lib/celestial/transforms-service";
import { createTimer, logError, wsLogger } from "@lib/logger";
import { decode, encode } from "@msgpack/msgpack";
import type { WebSocket } from "ws";
import { ZodError } from "zod";

import type { RequestInitWsType } from "./schema/Request/init.ws.model";
import type { RequestWsType } from "./schema/Request/request.ws.model";
import { requestWsSchema } from "./schema/Request/request.ws.model";
import type { RequestTransformWsType } from "./schema/Request/transform.ws.model";
import type { ResponseInitDataWsType } from "./schema/Response/init.ws.model";
import type { ResponseWsType } from "./schema/Response/response.ws.model";
import type { ResponseUpdateObjectDataWsType } from "./schema/Response/updateObject.ws.model";

// Store connected clients with metadata
const clients = new Map<WebSocket, { id: string; connectedAt: Date }>();

export const handleConnection = (ws: WebSocket) => {
  const clientId = generateClientId();

  clients.set(ws, {
    id: clientId,
    connectedAt: new Date(),
  });

  wsLogger.info({ clientId }, `🟢 Horizon connected`);

  sendMessage(ws, {
    type: "connected",
    clientId,
    timestamp: Date.now(),
  });

  // Incoming message management
  ws.on("message", (data) => handleMessage(ws, data));

  // Disconnection management
  ws.on("close", () => handleDisconnection(ws));

  // Error handling
  ws.on("error", (error) => handleError(ws, error));
};

const handleMessage = async (ws: WebSocket, data: unknown) => {
  const client = clients.get(ws);

  try {
    // Validate with Zod
    const msg: RequestWsType = requestWsSchema.parse(
      process.env.ENABLE_MSG_PACK === "true"
        ? decode(new Uint8Array(data as ArrayBuffer))
        : JSON.parse(data as string),
    );

    wsLogger.debug(
      {
        clientId: client?.id,
        event_type: msg.event_type,
      },
      "Message received",
    );

    await routeMessage(ws, msg);
  } catch (err: unknown) {
    handleMessageError(ws, err);
  }
};

const routeMessage = async (ws: WebSocket, msg: RequestWsType) => {
  switch (msg.event_type) {
    case "init":
      await handleInit(ws, msg);
      break;

    case "transform":
      handleTransform(ws, msg);
      break;
  }
};

const handleInit = async (ws: WebSocket, msg: RequestInitWsType) => {
  const timer = createTimer();
  const clientId = clients.get(ws)?.id;

  wsLogger.debug(
    {
      clientId,
      request: msg,
    },
    "Processing init request",
  );

  try {
    const objects = await getInit(msg.data);

    wsLogger.debug(
      {
        clientId,
        duration: timer.end(),
      },
      `✅ Sent init ${objects.length} objects`,
    );

    const initData: ResponseInitDataWsType[] = objects.map((object) => {
      return {
        object_type: object.objectType,
        object_uuid: object.target.uuid as string,
        object_data: {
          parent_id: object.parentId,
          from_timestamp: msg.data.from_timestamp,
          name: object.target.name,
          scenename:
            object.objectType !== "system"
              ? `scenes/${object.objectType}/${object.target.internalName}.tscn`
              : "",
          ...(object.soi && { soi: object.soi }),
          ...(object.transforms && {
            positions: object.transforms.map((transform) => transform.position),
            rotations: object.transforms.map((transform) => transform.rotation),
          }),
        },
      };
    });

    sendMessage(ws, {
      namespace: "genericprops",
      event: "create_object",
      data: initData,
    });
  } catch (error) {
    logError(wsLogger, error, { context: "handleInit" });
    sendError(
      ws,
      "Processing error",
      `${error instanceof Error ? error.message : "Failed to handle init"}`,
    );
  }

  wsLogger.debug(
    {
      clientId,
    },
    "Processing init request",
  );
};

const handleTransform = (ws: WebSocket, msg: RequestTransformWsType) => {
  const timer = createTimer();
  const clientId = clients.get(ws)?.id;

  wsLogger.debug(
    {
      clientId,
      request: msg,
    },
    "Processing transform request",
  );

  try {
    const object = getUpdateObject(msg.data);

    wsLogger.debug(
      {
        clientId,
        target: object.target.name,
        count: object.transforms?.length,
        duration: timer.end(),
      },
      `✅ Sent ${object.transforms?.length} positions`,
    );

    const responseUpdateObjectData: ResponseUpdateObjectDataWsType = {
      object_type: object.objectType,
      object_uuid: object.target.uuid as string,
      object_data: {
        from_timestamp: msg.data.from_timestamp,
        ...(object.transforms && {
          positions: object.transforms.map((transform) => transform.position),
          rotations: object.transforms.map((transform) => transform.rotation),
        }),
      },
    };

    sendMessage(ws, {
      namespace: "genericprops",
      event: "update_object",
      data: responseUpdateObjectData,
    });
  } catch (error) {
    logError(wsLogger, error, { context: "handleTransform" });
    sendError(
      ws,
      "Processing error",
      `${error instanceof Error ? error.message : "Failed to handle tranform"}`,
    );
  }
};

const handleMessageError = (ws: WebSocket, err: unknown) => {
  logError(wsLogger, err);

  const errorMessage =
    err instanceof ZodError
      ? err.issues
          .map((issue) => `On ${issue.path.join(".")} (${issue.message})`)
          .join(" | ")
      : String(err);

  sendError(ws, "Invalid message format", errorMessage);
};

const handleDisconnection = (ws: WebSocket) => {
  const client = clients.get(ws);

  if (client) {
    wsLogger.info({ clientId: client.id }, `🔴 Horizon disconnected`);
    clients.delete(ws);
  }
};

const handleError = (ws: WebSocket, error: Error) => {
  const client = clients.get(ws);
  logError(wsLogger, error, { context: "handleError", clientId: client?.id });
};

const sendMessage = (ws: WebSocket, data: ResponseWsType) => {
  if (ws.readyState === 1) {
    // OPEN
    ws.send(
      process.env.ENABLE_MSG_PACK === "true"
        ? encode(data)
        : JSON.stringify(data),
    );
  }
};

const sendError = (ws: WebSocket, error: string, message: string) => {
  sendMessage(ws, { error, message });
};

/**
 * Broadcast to all connected clients
 */
export const broadcast = (data: ResponseWsType, excludeWs?: WebSocket) => {
  clients.forEach((_client, ws) => {
    if (ws !== excludeWs && ws.readyState === 1) {
      sendMessage(ws, data);
    }
  });
};

/**
 * Generates a unique ID for a client
 */
const generateClientId = (): string => {
  return `client_${Date.now()}_${Math.random().toString(36).substring(7)}`;
};

/**
 * Retrieves the number of connected clients
 */
export const getClientsCount = () => clients.size;

/**
 * Retrieves information about all clients
 */
export const getClientsInfo = () => {
  return Array.from(clients.values());
};
