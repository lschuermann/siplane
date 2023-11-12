defmodule SiplaneWeb.WebUI.BoardController do
  use SiplaneWeb, :controller

  plug :put_view, html: SiplaneWeb.WebUI.PageHTML

  def index(conn, _params) do
    render(conn, :boards)
  end
end
