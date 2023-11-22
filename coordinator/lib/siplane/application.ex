defmodule Siplane.Application do
  # See https://hexdocs.pm/elixir/Application.html
  # for more information on OTP Applications
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    children = [
      # ##### Database

      # Start the Ecto database adapter repository
      Siplane.Repo,

      # ##### Web

      # Telemetry supervisor
      SiplaneWeb.Telemetry,
      {DNSCluster, query: Application.get_env(:siplane, :dns_cluster_query) || :ignore},
      {Phoenix.PubSub, name: Siplane.PubSub},

      # Start the Finch HTTP client for sending emails
      {Finch, name: Siplane.Finch},

      # Start a worker by calling: Siplane.Worker.start_link(arg)
      # {Siplane.Worker, arg},

      # Start to serve requests, typically the last entry
      SiplaneWeb.Endpoint,

      # ##### Business logic

      # Start a registry for boards and their server processes. This
      # is also used as a PubSub mechanism to subscribe to
      # board-related events.
      Siplane.Board,

      Siplane.Job,
    ]

    # See https://hexdocs.pm/elixir/Supervisor.html
    # for other strategies and supported options
    opts = [strategy: :one_for_one, name: Siplane.Supervisor]
    Supervisor.start_link(children, opts)
  end

  # Tell Phoenix to update the endpoint configuration
  # whenever the application is updated.
  @impl true
  def config_change(changed, _new, removed) do
    SiplaneWeb.Endpoint.config_change(changed, removed)
    :ok
  end

  # Move this somewhere else!
  def insert_dummy_data() do
    Siplane.Repo.insert!(
      %Siplane.Board{
	id: "ed8d3c39-6d34-41af-9fba-ff34109d9dbe",
	label: "Test nRF52840DK",
	location: "Princeton University",
	manufacturer: "Nordic Semiconductor",
	model: "nRF52840DK",
	runner_token: "foobar",
	image_url: "https://www.nordicsemi.com/-/media/Images/Products/DevKits/nRF52-Series/nRF52840-DK/nRF52840-DK-promo.png?sc_lang=en",
	environments: [
	  %Siplane.Environment{
	    id: "c4e08e00-cfb0-46d1-83eb-ee62e128cc70",
	    label: "Test Nix Nspawn Environment",
          }
	],
      }
    )

    Siplane.Repo.insert!(
      %Siplane.Job{
	id: "7556c6ae-d873-4246-9e06-e3a1712b02ff",
	label: "Test Job!!111eleven",
	start: DateTime.from_unix!(0),
	end: nil,
	dispatched: false,
	completion_code: nil,
	board_id: "ed8d3c39-6d34-41af-9fba-ff34109d9dbe",
	environment_id: "c4e08e00-cfb0-46d1-83eb-ee62e128cc70",
      }
    )
  end
end

