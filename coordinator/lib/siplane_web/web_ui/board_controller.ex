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

  defp update_board_state(socket) do
    ecto_board_id = Ecto.UUID.load!(socket.assigns.board_id)

    # Query board information, and get or start an orchestrator
    # process for this board, which we can then suscribe to to
    # receive log messages.
    socket =
      socket
      |> assign(board: Siplane.Repo.one!(
	  from b in Siplane.Board, where: b.id == ^ecto_board_id
	))
      |> assign(runner_connected: Siplane.Board.runner_connected?(socket.assigns.board_id))
      |> assign(pending_jobs: Siplane.Job.pending_jobs(board_id: socket.assigns.board_id, limit: 10))
      |> assign(active_jobs: Siplane.Job.active_jobs(board_id: socket.assigns.board_id))
      |> assign(completed_jobs: Siplane.Job.completed_jobs(board_id: socket.assigns.board_id, limit: 50))
  end

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

      {:ok, parsed_board_id} ->
	# Assign the board_id at the start. This way we can re-use the
	# update_board_state method to generate the board assign even
	# in the mount function:
	board_id = Keyword.get(parsed_board_id, :binary)
	socket = assign(socket, board_id: board_id)

	# Fetch the board state form the database & board server:
	socket = update_board_state(socket)

	# Subscribe to log messages
	:ok = Siplane.Board.subscribe board_id

	{
	  :ok,
	  assign(
	    socket,
	    board_id: board_id,
	    form: to_form(%{}),
	    log_events: Siplane.Board.board_log(board_id)
	  )
	}
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

  @impl true
  def handle_info({:board_event, _board_id, :update, _pld}, socket) do
    {:noreply, update_board_state(socket)}
  end

  @impl true
  def handle_info({:board_event, _board_id, _ev_type, _pld}, socket) do
    {:noreply, socket}
  end

  # @impl true
  # def handle_event("new-job", _, socket) do
  #   # IO.puts("New job!")

  #   # TODO: request creation of a proper job object which then
  #   # schedules itself on a board, etc. For now, lets just send a
  #   # message to the SSE channel quick and dirty.
  #   IO.puts "New code!"
  #   IO.inspect(socket.assigns.board_id)
  #   {:ok, _job_id} = Siplane.Board.new_instant_job(socket.assigns.board_id, "nixos_dev", "v1.0")

  #   {:noreply, socket}
  # end
end
