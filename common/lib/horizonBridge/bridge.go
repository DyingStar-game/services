package horizonBridge

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"sync"
	"time"

	"github.com/google/uuid"
	"google.golang.org/grpc"

	pb "dyingstar/services/common/lib/horizonBridge/gen/go/proto"
)

// EventHandler définit les callbacks pour les événements
type EventHandler struct {
	OnHorizonEvent func(category, eventType string, data map[string]interface{})
	OnPing         func()
	OnPong         func()
	OnError        func(error string)
	OnConnect      func()
	OnDisconnect   func()
}

// Event représente un événement à envoyer
type Event struct {
	Type      string                 `json:"type"`
	Category  string                 `json:"category"`
	Namespace string                 `json:"namespace,omitempty"`
	Plugin    string                 `json:"plugin,omitempty"`
	Data      map[string]interface{} `json:"data"`
}

// GRPCBridge encapsule le serveur gRPC avec interface événementielle
type GRPCBridge struct {
	bridgeImpl  *BridgeServer
	eventHandler *EventHandler
	
	// Gestion des streams pour diffusion
	streams   map[string]pb.BridgeService_EventStreamServer
	streamsMu sync.RWMutex
}

// BridgeServer implémente le service BridgeService
type BridgeServer struct {
	pb.UnimplementedBridgeServiceServer
	bridge *GRPCBridge
}

// NewGRPCBridge crée une nouvelle instance du bridge
func NewGRPCBridge(grpcServer *grpc.Server) (*GRPCBridge, error) {
	// Création de l'instance bridge
	bridge := &GRPCBridge{
		streams:  make(map[string]pb.BridgeService_EventStreamServer),
	}

	// Création du serveur gRPC
	bridgeImpl := &BridgeServer{bridge: bridge}
	
	// Enregistrement du service
	pb.RegisterBridgeServiceServer(grpcServer, bridgeImpl)

	bridge.bridgeImpl = bridgeImpl

	return bridge, nil
}

// SetEventHandler configure les callbacks d'événements
func (gb *GRPCBridge) SetEventHandler(handler *EventHandler) {
	gb.eventHandler = handler
}

// SendEvent envoie un événement à tous les clients connectés
func (gb *GRPCBridge) SendEvent(event Event) error {
	// Construction de la structure complète
	responseData := map[string]interface{}{
		"status":     "triggered",
		"timestamp":  time.Now().Unix(),
		"trigger_events": []Event{event},
	}
	
	responseJSON, err := json.Marshal(responseData)
	if err != nil {
		return fmt.Errorf("failed to marshal event: %v", err)
	}
	
	eventMsg := &pb.EventMessage{
		MessageId: uuid.New().String(),
		Type:      pb.EventType_GO_EVENT,
		EventJson: string(responseJSON),
		Timestamp: time.Now().Unix(),
		Source:    "go",
	}
	
	// Diffusion à tous les streams connectés
	gb.streamsMu.RLock()
	defer gb.streamsMu.RUnlock()
	
	success := 0
	for streamID, stream := range gb.streams {
		if err := stream.Send(eventMsg); err != nil {
			fmt.Printf("❌ Failed to send to stream %s: %v\n", streamID, err)
		} else {
			success++
		}
	}
	
	fmt.Printf("📤 Event sent to %d/%d streams\n", success, len(gb.streams))
	return nil
}

// SendEventToNamespace envoie un événement avec namespace spécifique
func (gb *GRPCBridge) SendEventToNamespace(eventType, namespace string, data map[string]interface{}) error {
	event := Event{
		Type:      eventType,
		Category:  "client",
		Namespace: namespace,
		Data:      data,
	}
	return gb.SendEvent(event)
}

// SendCoreEvent envoie un événement core
func (gb *GRPCBridge) SendCoreEvent(eventType string, data map[string]interface{}) error {
	event := Event{
		Type:     eventType,
		Category: "core",
		Data:     data,
	}
	return gb.SendEvent(event)
}

// SendPluginEvent envoie un événement plugin
func (gb *GRPCBridge) SendPluginEvent(eventType, plugin string, data map[string]interface{}) error {
	event := Event{
		Type:     eventType,
		Category: "plugin",
		Plugin:   plugin,
		Data:     data,
	}
	return gb.SendEvent(event)
}

// GetConnectedClients retourne le nombre de clients connectés
func (gb *GRPCBridge) GetConnectedClients() int {
	gb.streamsMu.RLock()
	defer gb.streamsMu.RUnlock()
	return len(gb.streams)
}

// Initialize - démarrage du bridge
func (s *BridgeServer) Initialize(ctx context.Context, req *pb.InitRequest) (*pb.InitResponse, error) {
	fmt.Printf("🚀 Bridge initialized for plugin: %s v%s\n", req.GetPluginName(), req.GetPluginVersion())
	
	return &pb.InitResponse{
		Message:       "Bridge ready",
		ServerVersion: "1.0.0-event-interface",
	}, nil
}

// EventStream - gestion du stream bidirectionnel
func (s *BridgeServer) EventStream(stream pb.BridgeService_EventStreamServer) error {
	streamID := uuid.New().String()
	fmt.Printf("🌊 New stream connected: %s\n", streamID)
	
	// Enregistrement du stream
	s.bridge.streamsMu.Lock()
	s.bridge.streams[streamID] = stream
	s.bridge.streamsMu.Unlock()
	
	// Callback de connexion
	if s.bridge.eventHandler != nil && s.bridge.eventHandler.OnConnect != nil {
		s.bridge.eventHandler.OnConnect()
	}
	
	// Nettoyage à la déconnexion
	defer func() {
		s.bridge.streamsMu.Lock()
		delete(s.bridge.streams, streamID)
		s.bridge.streamsMu.Unlock()
		
		fmt.Printf("🔌 Stream disconnected: %s\n", streamID)
		
		// Callback de déconnexion
		if s.bridge.eventHandler != nil && s.bridge.eventHandler.OnDisconnect != nil {
			s.bridge.eventHandler.OnDisconnect()
		}
	}()
	
	// Boucle principale pour recevoir les événements
	for {
		event, err := stream.Recv()
		if err == io.EOF {
			fmt.Printf("📡 Stream ended: %s\n", streamID)
			break
		}
		if err != nil {
			fmt.Printf("❌ Stream error for %s: %v\n", streamID, err)
			break
		}

		// Traiter l'événement reçu
		s.handleEvent(event, stream)
	}

	return nil
}

// handleEvent - traite les événements entrants
func (s *BridgeServer) handleEvent(event *pb.EventMessage, stream pb.BridgeService_EventStreamServer) {
	eventType := pb.EventType(event.GetType())
	
	switch eventType {
	case pb.EventType_HORIZON_EVENT:
		s.processHorizonEvent(event, stream)
		
	case pb.EventType_PING:
		fmt.Println("🏓 Ping received")
		s.sendPong(stream)
		if s.bridge.eventHandler != nil && s.bridge.eventHandler.OnPing != nil {
			s.bridge.eventHandler.OnPing()
		}
		
	case pb.EventType_PONG:
		fmt.Println("🏓 Pong received")
		if s.bridge.eventHandler != nil && s.bridge.eventHandler.OnPong != nil {
			s.bridge.eventHandler.OnPong()
		}
		
	case pb.EventType_ERROR:
		fmt.Printf("❌ Error received: %s\n", event.GetEventJson())
		if s.bridge.eventHandler != nil && s.bridge.eventHandler.OnError != nil {
			s.bridge.eventHandler.OnError(event.GetEventJson())
		}
		
	default:
		fmt.Printf("❓ Unknown event type: %d\n", int32(eventType))
	}
}

// processHorizonEvent - traite un événement Horizon
func (s *BridgeServer) processHorizonEvent(event *pb.EventMessage, stream pb.BridgeService_EventStreamServer) {
	// Parse l'événement JSON
	var eventData map[string]interface{}
	if err := json.Unmarshal([]byte(event.GetEventJson()), &eventData); err != nil {
		fmt.Printf("❌ Error parsing event: %v\n", err)
		return
	}

	// Extraction des infos de base
	category := getString(eventData, "category")
	eventType := getString(eventData, "event")
	
	fmt.Printf("📥 Processing: %s -> %s\n", category, eventType)

	// Appel du callback utilisateur
	if s.bridge.eventHandler != nil && s.bridge.eventHandler.OnHorizonEvent != nil {
		// Extraction des données de l'événement
		var data map[string]interface{}
		if eventDataField, ok := eventData["data"]; ok {
			if dataMap, ok := eventDataField.(map[string]interface{}); ok {
				data = dataMap
			}
		}
		if data == nil {
			data = make(map[string]interface{})
		}
		
		// Ajout des métadonnées
		data["namespace"] = getString(eventData, "namespace")
		data["plugin"] = getString(eventData, "plugin")
		data["raw_event"] = eventData
		
		s.bridge.eventHandler.OnHorizonEvent(category, eventType, data)
	}
}

// sendPong - répond au ping
func (s *BridgeServer) sendPong(stream pb.BridgeService_EventStreamServer) {
	pong := &pb.EventMessage{
		MessageId: uuid.New().String(),
		Type:      pb.EventType_PONG,
		Timestamp: time.Now().Unix(),
		Source:    "go",
	}
	
	stream.Send(pong)
}

// HealthCheck - vérification de santé
func (s *BridgeServer) HealthCheck(ctx context.Context, req *pb.HealthRequest) (*pb.HealthResponse, error) {
	clientCount := s.bridge.GetConnectedClients()
	fmt.Printf("💚 Health check - %d clients connected\n", clientCount)
	
	return &pb.HealthResponse{
		Status: fmt.Sprintf("healthy - %d clients", clientCount),
	}, nil
}

// Shutdown - fermeture propre
func (s *BridgeServer) Shutdown(ctx context.Context, req *pb.ShutdownRequest) (*pb.ShutdownResponse, error) {
	fmt.Println("🔌 Shutdown requested")
	
	return &pb.ShutdownResponse{
		Message: "Shutting down",
	}, nil
}

// Fonction utilitaire pour extraire les strings
func getString(data map[string]interface{}, key string) string {
	if val, ok := data[key]; ok {
		if str, ok := val.(string); ok {
			return str
		}
	}
	return ""
}