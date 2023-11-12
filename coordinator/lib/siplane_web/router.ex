defmodule SiplaneWeb.Router do
  use SiplaneWeb, :router

  pipeline :browser do
    plug :accepts, ["html"]
    plug :fetch_session
    plug :fetch_live_flash
    plug :put_root_layout, html: {SiplaneWeb.WebUI.Layouts, :root}
    plug :protect_from_forgery
    plug :put_secure_browser_headers
    plug SiplaneWeb.WebUI.Plugs.SetUser
  end

  pipeline :api do
    plug :accepts, ["json"]
  end

  scope "/", SiplaneWeb do
    pipe_through :browser

    get "/", WebUI.PageController, :home
    get "/boards", WebUI.BoardController, :index
  end

  scope "/auth", SiplaneWeb do
    pipe_through :browser

    # Specific routes, to avoid hitting the generic :provider match
    get "/logout", WebUI.AuthController, :logout

    get "/:provider", WebUI.AuthController, :request
    get "/:provider/callback", WebUI.AuthController, :callback
  end

  scope "/api/runner/v0", SiplaneWeb do
    pipe_through :api

    resources "/boards", API.Runner.V0.BoardController
    get "/boards/:id/sse", API.Runner.V0.BoardController, :sse_conn
  end

  # Other scopes may use custom stacks.
  # scope "/api", SiplaneWeb do
  #   pipe_through :api
  # end

  # Enable LiveDashboard and Swoosh mailbox preview in development
  if Application.compile_env(:siplane, :dev_routes) do
    # If you want to use the LiveDashboard in production, you should put
    # it behind authentication and allow only admins to access it.
    # If your application does not have an admins-only section yet,
    # you can use Plug.BasicAuth to set up some basic authentication
    # as long as you are also using SSL (which you should anyway).
    import Phoenix.LiveDashboard.Router

    scope "/dev" do
      pipe_through :browser

      live_dashboard "/dashboard", metrics: SiplaneWeb.Telemetry
      forward "/mailbox", Plug.Swoosh.MailboxPreview
    end
  end
end
