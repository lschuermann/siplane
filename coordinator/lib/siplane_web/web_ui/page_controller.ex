defmodule SiplaneWeb.WebUI.PageController do
  use SiplaneWeb, :controller

  plug :put_view, html: SiplaneWeb.WebUI.PageHTML

  def home(conn, _params) do
    # The home page is often custom made,
    # so skip the default app layout.
    render(conn, :home, layout: false)
  end
end
