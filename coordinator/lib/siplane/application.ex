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

      # Start a registry for board orchestrators managing runner connections & boards generally
      {Registry, keys: :unique, name: Siplane.BoardOrchestrator.Registry},
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
end
