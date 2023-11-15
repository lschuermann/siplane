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

  @impl true
  def render(assigns) do
    SiplaneWeb.WebUI.PageHTML.board(assigns)
  end

  @impl true
  def handle_params(_params, uri, socket) do
    {:noreply, assign(socket, current_uri: uri)}
  end

  @impl true
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

	# Query board information, and get or start an orchestrator
	# process for this board, which we can then suscribe to to
	# receive log messages.
	board = Siplane.Repo.one!(
	  from b in Siplane.Board, where: b.id == ^binary_board_id)

	board = Map.put(
	  board,
	  :runner_connected,
	  Siplane.Board.runner_connected?(binary_board_id)
	)

	# Subscribe to log messages
	:ok = Siplane.Board.subscribe binary_board_id

	socket = assign(socket, board: board)

	socket = assign(
	  socket,
	  log_events: Siplane.Board.board_log(binary_board_id),
	)

	{:ok, socket}
    end
  end

  @impl true
  def handle_info({:board_event, _board_id, :log_event, event}, socket) do
    {
      :noreply,
      assign(
	socket,
	log_events: [ event | socket.assigns.log_events ]
      )
    }
  end

end
