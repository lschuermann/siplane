defmodule SiplaneWeb.WebUI.AuthController do
  use SiplaneWeb, :controller
  import Ecto.Query

  # Important to use the :github_custom provider here, as it avoids catching the
  # /auth/github request and allows us to use a custom request handler that also
  # sets some cookies.
  plug Ueberauth, providers: [:github_custom]

  @github_provider_config {
    Ueberauth.Strategy.Github, [
      default_scope: "user:email,read:public_key",
    ]
  }

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

  # Initial request route to kick off the SSO flow
  def request(conn, %{"provider" => "github"} = params) do
    IO.inspect(conn)
    IO.inspect(params)

    conn
    # Important to clear the redirect_url if we don't have one, such that we
    # don't use a previously set redirect_url.
    |> put_session(:post_auth_redirect, Map.get(params, "redirect_url", ""))
    |> Ueberauth.run_request("github", @github_provider_config)
  end

  # Callback for the Uberauth SSO flows
  def callback(conn, %{"provider" => "github"} = params) do
    # user_data = %{token: auth.credentials.token, email: auth.info.email, provider: "github"}
    %{assigns: %{ueberauth_auth: auth}} =
      conn
      |> Ueberauth.run_callback("github", @github_provider_config)

    {:ok, user} = find_or_create_user_with_provider(auth.info.name, auth.info.email, "github", auth.credentials.token)

    IO.inspect(conn)
    post_auth_redirect_state =
      get_session(conn, :post_auth_redirect, "")
    IO.puts("Got from session: #{inspect post_auth_redirect_state}")

    post_auth_redirect =
      if post_auth_redirect_state <> "" do
	post_auth_redirect_state
      else
	"/"
      end

    conn
    |> put_flash(:info, "Successfully signed in!")
    |> put_session(:user_id, user.id)
    |> redirect(external: post_auth_redirect)
  end

  def logout(conn, _params) do
    conn
    |> configure_session(drop: true)
    |> redirect(to: "/")
  end
end
