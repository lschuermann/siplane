defmodule TreadmillWeb.WebUI.LiveHooks.SetUser do
  import Phoenix.Component
  import Phoenix.LiveView

  alias Treadmill.Repo
  alias Treadmill.User

  def on_mount(:default, _params, session, socket) do
    socket = assign_new(socket, :user, fn ->
      IO.puts("Evaluating socket user assign, session: #{inspect session}!")
      user_id = Map.get session, "user_id"
      cond do
	user = user_id && Repo.get(User, user_id) ->
	  user
	true ->
	  nil
      end
    end)

    {:cont, socket}
  end
end
