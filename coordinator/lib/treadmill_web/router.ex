defmodule TreadmillWeb.Router do
  use TreadmillWeb, :router

  pipeline :browser do
    plug :accepts, ["html"]
    plug :fetch_session
    plug :fetch_live_flash
    plug :put_root_layout, html: {TreadmillWeb.WebUI.Layouts, :root}
    plug :protect_from_forgery
    plug :put_secure_browser_headers
    plug TreadmillWeb.WebUI.Plugs.SetUser
  end

  pipeline :api do
    plug :accepts, ["json", "sse"]
  end

  scope "/", TreadmillWeb do
    pipe_through :browser

    live_session :default, on_mount: TreadmillWeb.WebUI.LiveHooks.SetUser do
      live "/jobs/:id", WebUI.JobController.Live
      live "/boards/:id", WebUI.BoardController.Live
    end


    get "/", WebUI.PageController, :home
    resources "/boards", WebUI.BoardController, only: [:index, :show]
    get "/user", WebUI.UserController, :index
  end

  scope "/auth", TreadmillWeb do
    pipe_through :browser

    # Specific routes, to avoid hitting the generic :provider match
    get "/logout", WebUI.AuthController, :logout

    get "/:provider", WebUI.AuthController, :request
    get "/:provider/callback", WebUI.AuthController, :callback
  end

  scope "/api/runner/v0", TreadmillWeb do
    pipe_through :api

    put "/boards/:id/state", API.Runner.V0.BoardController, :update_state
    get "/boards/:id/sse", API.Runner.V0.BoardController, :sse_conn

    put "/jobs/:id/state", API.Runner.V0.JobController, :update_state
    put "/jobs/:id/console", API.Runner.V0.JobController, :put_console_log
  end

  # Other scopes may use custom stacks.
  # scope "/api", TreadmillWeb do
  #   pipe_through :api
  # end

  # Enable LiveDashboard and Swoosh mailbox preview in development
  if Application.compile_env(:treadmill, :dev_routes) do
    # If you want to use the LiveDashboard in production, you should put
    # it behind authentication and allow only admins to access it.
    # If your application does not have an admins-only section yet,
    # you can use Plug.BasicAuth to set up some basic authentication
    # as long as you are also using SSL (which you should anyway).
    import Phoenix.LiveDashboard.Router

    scope "/dev" do
      pipe_through :browser

      live_dashboard "/dashboard", metrics: TreadmillWeb.Telemetry
      forward "/mailbox", Plug.Swoosh.MailboxPreview
    end
  end
end
