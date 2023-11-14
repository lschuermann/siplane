defmodule SiplaneWeb.WebUI.UserController do
  use SiplaneWeb, :controller

  plug :put_view, html: SiplaneWeb.WebUI.PageHTML

  def index(conn, _params) do
    case conn.assigns.user do
      nil ->
	conn
	|> put_status(401)
	|> put_view(SiplaneWeb.WebUI.ErrorHTML)
	|> render("401.html")
      user ->
	render(conn, :user, %{ user: user })
    end
  end
end
