defmodule TreadmillWeb.API.Runner.V0.BoardController do
  use TreadmillWeb, :controller
  require Logger

  @sse_keepalive_timeout 15000

  # def update_state(conn, %{"id" => board_id_str}) do
  #   # TODO: auth, validate payload, etc.
  #   case UUID.info(board_id_str) do
  #     {:error, _} ->
  # 	conn
  # 	|> put_status(:bad_request)
  # 	|> put_view(TreadmillWeb.API.ErrorView)
  # 	|> render("bad_request.json",
  # 	  description: "Provided board ID is not a valid UUID."
  # 	)

  #     {:ok, parsed_board_id} ->
  # 	board_id = Keyword.get(parsed_board_id, :binary)

  # 	case conn.body_params["state"] do
  # 	  "idle" ->
  # 	    Treadmill.Board.update_state(board_id, :idle)
  # 	    send_resp(conn, 204, "")
  # 	end
  #   end
  # end

  def sse_conn(%{adapter: {Plug.Cowboy.Conn, cowboy_req}} = conn, %{"id" => board_id_str}) do
    case UUID.info(board_id_str) do
      {:error, _} ->
	conn
	|> put_status(:bad_request)
	|> put_view(TreadmillWeb.API.ErrorView)
	|> render("bad_request.json",
	  description: "Provided board ID is not a valid UUID."
	)

      {:ok, parsed_board_id} ->
	board_id = Keyword.get(parsed_board_id, :binary)

	# TODO: check that this board actually exist and validate the
	# token provided by the runner...

	# TODO: validate the auth connection

	# With the token validated, we can set this request to never
	# time out. We should still limit each board to one open SSE
	# conn.
	:cowboy_req.cast({:set_options, %{ idle_timeout: :infinity }}, cowboy_req)


	conn = conn
	|> put_resp_header("Cache-Control", "no-cache")
	|> put_resp_header("Connection", "keep-alive")
	|> put_resp_header("Content-Type", "text/event-stream; charset=utf-8")
	|> send_chunked(200)

	# Register this request handler process as a connection for
	# this runner to the orchestrator:
	{:ok, board_pid, last_will_and_testament} =
	  Treadmill.Board.connect_runner board_id, %{
	    ip: "TODO get client IP",
	    port: 1234,
	  }

	# send(board_pid, nil)

	# We want to close the SSE connection when the board server
	# dies. Thus monitor this process. This will still deliver a
	# message even if the process is already dead:
	Process.monitor board_pid

	# Setup a keep-alive timer for SSE. This timer will be reset
	# every time a message is sent or the timer fires.
	timer = Process.send_after(self(), :sse_keepalive, @sse_keepalive_timeout)

	# Receive runner messages and forward them to SSE in a loop:
	sse_loop conn, board_pid, board_id, last_will_and_testament, timer
    end
  end

  defp sse_loop(conn, board_pid, board_id, last_will_and_testament, timer) do
    {cont, timer_action} = receive do
      {:board_event, ^board_id, :runner_msg, msg} ->
	# Send message:
	chunk(conn, "event: message\ndata: #{Jason.encode! msg}\n\n")
	{true, :reset}

      {:DOWN, _ref, :process, ^board_pid, _type} ->
	# When the orchestrator goes down, we should close SSE
	chunk(conn, "event: close\ndata: #{Jason.encode! last_will_and_testament}\n\n")
	{false, :cancel}

      :sse_keepalive ->
	chunk(conn, ":\n\n")
	{true, :reset}

      _other ->
	# Ignore any unrelated messages:
	{true, nil}
    end

    timer =
      case timer_action do
	:cancel ->
	  Process.cancel_timer(timer)
	  timer

      :reset ->
	  Process.cancel_timer(timer)
	  Process.send_after(self(), :sse_keepalive, @sse_keepalive_timeout)

	_ ->
	  timer
      end

    if cont do
      sse_loop conn, board_pid, board_id, last_will_and_testament, timer
    else
      conn
    end
  end


end
