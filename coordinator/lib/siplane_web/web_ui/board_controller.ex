defmodule SiplaneWeb.WebUI.BoardController do
  use SiplaneWeb, :controller
  use Phoenix.LiveView
  import Ecto.Query

  plug :put_view, html: SiplaneWeb.WebUI.PageHTML

  def render(assigns) do
    render(SiplaneWeb.WebUI.PageHTML, "board.html", assigns)
  end

  def index(conn, _params) do
    boards = Siplane.Repo.all(Siplane.Board)
    |> Enum.map(fn board ->
      Map.from_struct(board)
      |> Map.put(:runner_connected, Siplane.Board.runner_connected?(board.id))
    end)

    conn
    |> render(:boards, %{ boards: boards })
  end

  def show(conn, %{"id" => board_id_str} = _params) do
    case UUID.info(board_id_str) do
      {:error, _} ->
	conn
	|> put_status(:bad_request)
	|> put_view(SiplaneWeb.WebUI.ErrorHTML)
	|> render("400.html")

      {:ok, parsed_board_id} ->
	binary_board_id =
	  Keyword.get(parsed_board_id, :binary)
  	  |> Ecto.UUID.load!

	board = Siplane.Repo.one!(
	  from b in Siplane.Board, where: b.id == ^binary_board_id)

	board = Map.put board, :runner_connected, Siplane.Board.runner_connected?(board.id)

	conn
	|> render(:board, %{ board: board })
    end
  end
end

defmodule SiplaneWeb.WebUI.BoardController.Live do
  use SiplaneWeb, :live_view
  import Ecto.Query

  def render(assigns) do
    # ~H"""
    #     Hello <%= inspect @user %>
    # """
    SiplaneWeb.WebUI.PageHTML.board(assigns)
  end

  def mount(%{"id" => board_id_str} = _params, _session, socket) do
    case UUID.info(board_id_str) do
      {:error, _} ->
	{:error, :uuid_invalid}
	# conn
	# |> put_status(:bad_request)
	# |> put_view(SiplaneWeb.WebUI.ErrorHTML)
	# |> render("400.html")

      {:ok, parsed_board_id} ->
	binary_board_id =
	  Keyword.get(parsed_board_id, :binary)
  	  |> Ecto.UUID.load!

	board = Siplane.Repo.one!(
	  from b in Siplane.Board, where: b.id == ^binary_board_id)

	board = Map.put board, :runner_connected, Siplane.Board.runner_connected?(board.id)

	socket = assign(socket, board: board)

	{:ok, socket}
    end
  end
end
