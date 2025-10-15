package server

import (
	"context"
	br "dyingstar/services/common/lib/horizonBridge"
	"dyingstar/services/persistance/app/config"
	"fmt"
	"log"
	"net"
	"os"
	"os/signal"
	"strconv"
	"syscall"
	"time"

	"google.golang.org/grpc"
)

var server *grpc.Server
var bridge *br.GRPCBridge

func Start() {

	cfg := config.GetConfig()
	// Configuration du port
	port := strconv.Itoa(cfg.Server.Port)

	// Création du listener
	lis, err := net.Listen("tcp", ":"+port)
	if err != nil {
		log.Fatalf("failed to listen on %s: %v", port, err)
	}

	server = grpc.NewServer(
		grpc.UnaryInterceptor(loggingUnaryInterceptor),
		grpc.StreamInterceptor(loggingStreamInterceptor),
	)

	// Création du bridge
	bridge, err = br.NewGRPCBridge(server)
	if err != nil {
		log.Fatalf("Failed to create bridge: %v", err)
	}

	// Configuration des callbacks d'événements
	bridge.SetEventHandler(&br.EventHandler{
		OnConnect: func() {
			fmt.Printf("🔗 Client connected! Total: %d\n", bridge.GetConnectedClients())

			// Envoie un message de bienvenue
			bridge.SendEventToNamespace("welcome", "notifications", map[string]interface{}{
				"message": "Bienvenue! Vous êtes connecté au server Go!",
				"time":    time.Now().Format("15:04:05"),
			})
		},

		OnDisconnect: func() {
			fmt.Printf("🔌 Client disconnected! Remaining: %d\n", bridge.GetConnectedClients())
		},

		OnHorizonEvent: func(category, eventType string, data map[string]interface{}) {
			fmt.Printf("🎯 Event reçu: %s/%s\n", category, eventType)

			// Exemple: répondre aux messages de chat
			if category == "client" && eventType == "message_sent" {
				if message, ok := data["message"].(string); ok {
					bridge.SendEventToNamespace("chat_response", "chat", map[string]interface{}{
						"message": fmt.Sprintf("Echo: %s", message),
						"sender":  "go_bot",
					})
				}
			}

			// Exemple: répondre aux événements de login
			if category == "client" && eventType == "login" {
				if username, ok := data["username"].(string); ok {
					bridge.SendCoreEvent("user_logged_in", map[string]interface{}{
						"username": username,
						"message":  "Utilisateur connecté avec succès",
					})
				}
			}
		},

		OnPing: func() {
			fmt.Println("💓 Heartbeat reçu")
		},

		OnError: func(error string) {
			fmt.Printf("⚠️ Erreur reçue: %s\n", error)
		},
	})

	// Démarrage du serveur
	go func() {
		fmt.Printf("🌊 gRPC starting on %s\n", lis.Addr().String())
		fmt.Println("Press Ctrl+C to stop")

		if err := server.Serve(lis); err != nil {
			log.Printf("Failed to serve gRPC: %v", err)
		}
	}()

	// Exemple: envoyer des événements périodiques
	go func() {
		ticker := time.NewTicker(30 * time.Second)
		defer ticker.Stop()

		for {
			<-ticker.C
			bridge.SendEventToNamespace("periodic_notification", "notifications", map[string]interface{}{
				"message": "Serveur Go opérationnel",
				"uptime":  time.Now().Unix(),
				"clients": bridge.GetConnectedClients(),
			})
		}
	}()

	// Attente de l'arrêt
	waitForShutdown()
	fmt.Println("👋 shutdown complete")
}

// Stop arrête proprement le serveur
func stop() {
	fmt.Println("\n🔌 Stopping gRPC ...")

	// Arrêt gracieux avec timeout
	stopped := make(chan struct{})
	go func() {
		server.GracefulStop()
		close(stopped)
	}()

	// Timeout de 3 secondes pour l'arrêt gracieux
	select {
	case <-stopped:
		fmt.Println("✅ gRPC stopped gracefully")
	case <-time.After(3 * time.Second):
		fmt.Println("⚠️ Graceful shutdown timeout, forcing stop...")
		server.Stop() // Arrêt forcé
		fmt.Println("✅ gRPC stopped (forced)")
	}
}

// WaitForShutdown attend les signaux d'arrêt (Ctrl+C, etc.)
func waitForShutdown() {
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)

	sig := <-sigChan
	fmt.Printf("\n📡 Received signal: %v\n", sig)
	stop()

	// Force l'arrêt si nécessaire après 5 secondes
	go func() {
		time.Sleep(5 * time.Second)
		fmt.Println("⚠️ Force shutdown after timeout")
		os.Exit(1)
	}()
}

func loggingUnaryInterceptor(
	ctx context.Context,
	req interface{},
	info *grpc.UnaryServerInfo,
	handler grpc.UnaryHandler,
) (resp interface{}, err error) {
	log.Printf("gRPC Unary call: %s", info.FullMethod)
	return handler(ctx, req)
}

// Stream interceptor pour logger tous les appels stream
func loggingStreamInterceptor(
	srv interface{},
	ss grpc.ServerStream,
	info *grpc.StreamServerInfo,
	handler grpc.StreamHandler,
) error {
	log.Printf("gRPC Stream call: %s", info.FullMethod)
	return handler(srv, ss)
}
