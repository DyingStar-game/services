/*
Copyright © 2025 NAME HERE <EMAIL ADDRESS>
*/
package cmd

import (
	"dyingstar/services/persistance/domain/models"
	datarepository "dyingstar/services/persistance/infra/database/repository"
	"fmt"

	"github.com/spf13/cobra"
)

// devTestCmd represents the devTest command
var devTestCmd = &cobra.Command{
	Use:   "devTest",
	Short: "A brief description of your command",
	Long: `A longer description that spans multiple lines and likely contains examples
and usage of using your command. For example:

Cobra is a CLI library for Go that empowers applications.
This application is a tool to generate the needed files
to quickly create a Cobra application.`,
	Run: func(cmd *cobra.Command, args []string) {
		fmt.Println("devTest called")
		/*sysuid, _ := uuid.NewUUID()
		sytem := models.NewSystem(sysuid.String(), "testSystem")
		ptuid, _ := uuid.NewUUID()
		planet := models.NewPlanet(
			&sytem, ptuid.String(),
			"testPlantet",
			models.Position{
				X: 2,
				Y: 2,
				Z: 2,
			},
		)*/
		repository := datarepository.NewCommonRepository[*models.Planet]()
		planet := repository.LoadByUId("0x9")
		//repository.Save(&planet)
		fmt.Println(planet)
		/*player := models.NewPlayer()
		player.Parent = &planet.Entity
		prepo := datarepository.NewPlayerRepository()
		prepo.Save(&player)
		fmt.Println(player)*/
	},
}

func init() {
	rootCmd.AddCommand(devTestCmd)

	// Here you will define your flags and configuration settings.

	// Cobra supports Persistent Flags which will work for this command
	// and all subcommands, e.g.:
	// devTestCmd.PersistentFlags().String("foo", "", "A help for foo")

	// Cobra supports local flags which will only run when this command
	// is called directly, e.g.:
	// devTestCmd.Flags().BoolP("toggle", "t", false, "Help message for toggle")
}
