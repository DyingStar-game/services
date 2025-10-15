/*
Copyright © 2025 NAME HERE <EMAIL ADDRESS>
*/
package cmd

import (
	"dyingstar/services/persistance/infra/database"

	"github.com/spf13/cobra"
)

var databaseCmd = &cobra.Command{
	Use:   "database",
	Short: "Database management commands",
	Long:  "Manage database operations such as initialization, wiping, and resetting.",
}

// Sous-commande : `myapp database init`
var databaseInitCmd = &cobra.Command{
	Use:   "init",
	Short: "Initialize the database",
	Run: func(cmd *cobra.Command, args []string) {
		database.Init()
	},
}

// Sous-commande : `myapp database wipe`
var databaseWipeCmd = &cobra.Command{
	Use:   "wipe",
	Short: "Wipe the database",
	Run: func(cmd *cobra.Command, args []string) {
		database.Wipe()
	},
}

// Sous-commande : `myapp database reset`
var databaseResetCmd = &cobra.Command{
	Use:   "reset",
	Short: "Reset the database",
	Run: func(cmd *cobra.Command, args []string) {
		database.ResetData()
	},
}

func init() {
	// Ajouter les sous-commandes à `database`
	databaseCmd.AddCommand(databaseInitCmd)
	databaseCmd.AddCommand(databaseWipeCmd)
	databaseCmd.AddCommand(databaseResetCmd)

	// Ajouter `database` à la racine
	rootCmd.AddCommand(databaseCmd)
}
