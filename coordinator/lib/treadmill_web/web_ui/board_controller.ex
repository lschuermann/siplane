defmodule TreadmillWeb.WebUI.BoardController do
  use TreadmillWeb, :controller
  use Phoenix.LiveView
  import Ecto.Query

  plug :put_view, html: TreadmillWeb.WebUI.PageHTML

  def render(assigns) do
    render(TreadmillWeb.WebUI.PageHTML, "board.html", assigns)
  end

  def index(conn, _params) do
    boards = Treadmill.Repo.all(Treadmill.Board)
    |> Enum.map(fn board ->
      Map.from_struct(board)
      |> Map.put(:runner_connected, Treadmill.Board.runner_connected?(board.id))
    end)

    conn
    |> render(:boards, %{ boards: boards })
  end

  def show(conn, %{"id" => board_id_str} = _params) do
    case UUID.info(board_id_str) do
      {:error, _} ->
	conn
	|> put_status(:bad_request)
	|> put_view(TreadmillWeb.WebUI.ErrorHTML)
	|> render("400.html")

      {:ok, parsed_board_id} ->
	binary_board_id =
	  Keyword.get(parsed_board_id, :binary)
  	  |> Ecto.UUID.load!

	board = Treadmill.Repo.one!(
	  from b in Treadmill.Board, where: b.id == ^binary_board_id)

	board = Map.put board, :runner_connected, Treadmill.Board.runner_connected?(board.id)

	conn
	|> render(:board, %{ board: board })
    end
  end
end

defmodule TreadmillWeb.WebUI.BoardController.Live do
  use TreadmillWeb, :live_view
  import Ecto.Query

  defp update_board_state(socket) do
    ecto_board_id = Ecto.UUID.load!(socket.assigns.board_id)

    # Query board information, and get or start an orchestrator
    # process for this board, which we can then suscribe to to
    # receive log messages.
    socket =
      socket
      |> assign(board: (
	  Treadmill.Repo.one!(
	    from b in Treadmill.Board, where: b.id == ^ecto_board_id
	  )
	  |> Treadmill.Repo.preload(:environments)
	))
      |> assign(runner_connected: Treadmill.Board.runner_connected?(socket.assigns.board_id))
      |> assign(pending_jobs: Treadmill.Job.pending_jobs(board_id: socket.assigns.board_id, limit: 5))
      |> assign(active_jobs: Treadmill.Job.active_jobs(board_id: socket.assigns.board_id))
      |> assign(completed_jobs: Treadmill.Job.completed_jobs(board_id: socket.assigns.board_id, limit: 10))
  end

  @impl true
  def render(assigns) do
    TreadmillWeb.WebUI.PageHTML.board(assigns)
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
	:ok = Treadmill.Board.subscribe board_id

	{
	  :ok,
	  assign(
	    socket,
	    board_id: board_id,
	    create_job_form: to_form(%{ "environment_id" => "", "label" => "" }),
	    log_events: Treadmill.Board.board_log(board_id)
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

  @impl true
  def handle_event("save_create_job", params, socket) do
    { :noreply, assign(socket, create_job_form: to_form(params)) }
  end

  @impl true
  def handle_event("create_job", %{"environment_id" => environment_id_str, "label" => job_label} = params, socket) do
    # First, save the form params. If we're returning an error, we
    # don't want to clear the contents:
    socket = assign(socket, create_job_form: to_form(params))

    with {:ok, parsed_env_id} <- UUID.info(environment_id_str),
         env_id = Keyword.get(parsed_env_id, :binary),
         ecto_env_id = Ecto.UUID.load!(env_id),
	 true <- Treadmill.Repo.exists?(from(e in Treadmill.Environment, where: e.id == ^ecto_env_id)),
         %Treadmill.User{id: ecto_user_id} <- Map.get(socket.assigns, :user, nil), # If not logged in, socket.assigns == nil
         {:ok, parsed_user_id} = UUID.info(ecto_user_id),
         user_id = Keyword.get(parsed_user_id, :binary) do

      # Start job:
      job_label = if job_label != "", do: job_label, else: nil
      {:ok, job} = Treadmill.Job.instant_job(socket.assigns.board_id, env_id, job_label, user_id)

      {
	:noreply,
	socket
	|> put_flash(:info, "Started job!")
	|> push_navigate(to: "/jobs/#{job.id}")
      }
    else
      {:error, _} ->
	{:noreply, put_flash(socket, :error, "Select valid environment!")}
      false ->
	{:noreply, put_flash(socket, :error, "Select valid environment!")}
      nil ->
	{:noreply, put_flash(socket, :error, "Log in first!")}
    end
  end

  @impl true
  def handle_event("create_job", params, socket) do
    socket =
      socket
      |> put_flash(:error, "Select a valid job environment to start a new job!")
      |> assign(create_job_form: to_form(params))
    {:noreply, socket}
  end


    # # TODO: request creation of a proper job object which then
    # # schedules itself on a board, etc. For now, lets just send a
    # # message to the SSE channel quick and dirty.
    # IO.puts "New code!"
    # IO.inspect(socket.assigns.board_id)
    # {:ok, _job_id} = Treadmill.Board.new_instant_job(socket.assigns.board_id, "nixos_dev", "v1.0")

    # {:noreply, socket}

end
