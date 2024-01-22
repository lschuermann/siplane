defmodule TreadmillWeb.WebUI.UserController do
  use TreadmillWeb, :controller

  plug :put_view, html: TreadmillWeb.WebUI.PageHTML

  def index(conn, _params) do
    case conn.assigns.user do
      nil ->
	conn
	|> put_status(401)
	|> put_view(TreadmillWeb.WebUI.ErrorHTML)
	|> render("401.html")
      user ->
	render(conn, :user, %{ user: user })
    end
  end
end
