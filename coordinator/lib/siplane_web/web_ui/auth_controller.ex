defmodule SiplaneWeb.WebUI.AuthController do
  use SiplaneWeb, :controller
  import Ecto.Query
  plug Ueberauth

  # TODO: should this be in the User module instead?
  defp find_or_create_user_with_provider(name, email, provider, token) do
    user = Siplane.Repo.insert!(
      Siplane.User.changeset(
	%Siplane.User{},
	%{
	  name: name,
	  email: email
	}
      ),
      # This is functionally equivalent to `on_conflict: nothing`, but forces
      # the correct record to be loaded from the database (e.g. correct
      # `user.id` field). Otherwise, this will just invent another record and
      # return it, but not actually persist it.
      on_conflict: [set: [email: email]],
      conflict_target: :email,
      returning: true
    )

    Siplane.Repo.insert!(
      Siplane.User.UserProvider.changeset(
	%Siplane.User.UserProvider{},
	%{
	  user_id: user.id,
	  provider: provider,
	  token: token,
	}
      ),
      # Always refresh the token in the DB, should we be provided a new one:
      on_conflict: [set: [token: token]],
      conflict_target: [:user_id, :provider]
    )

    {:ok, user}
  end

  # Callback for the Uberauth SSO flows
  def callback(%{assigns: %{ueberauth_auth: auth}} = conn, params) do
    # user_data = %{token: auth.credentials.token, email: auth.info.email, provider: "github"}
    {:ok, user} = find_or_create_user_with_provider(auth.info.name, auth.info.email, "github", auth.credentials.token)

    conn
    |> put_flash(:info, "Successfully signed in!")
    |> put_session(:user_id, user.id)
    |> redirect(to: "/")
  end

  def logout(conn, _params) do
    conn
    |> configure_session(drop: true)
    |> redirect(to: "/")
  end
end
