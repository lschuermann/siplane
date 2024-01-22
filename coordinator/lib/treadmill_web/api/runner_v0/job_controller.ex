defmodule TreadmillWeb.API.Runner.V0.JobController do
  use TreadmillWeb, :controller
  require Logger

  def cast_uuid_string_binary(uuid) do
    case UUID.info(uuid) do
      {:error, _} -> {:error, "Invalid UUID string #{uuid}"}
      {:ok, parsed_uuid} -> {:ok, Keyword.get(parsed_uuid, :binary) }
    end
  end

  @job_id_params_schema %{
    id: [type: :string, required: true, cast_func: &__MODULE__.cast_uuid_string_binary/1],
  }

  def cast_job_state(state) do
    case state do
      "starting" -> {:ok, :starting}
      "ready" -> {:ok, :ready}
      "stopping" -> {:ok, :stopping}
      "finished" -> {:ok, :finished}
      "failed" -> {:ok, :failed}
      _ -> {:error, "Invalid state #{state}"}
    end
  end

  def cast_job_starting_stage(stage, data) do
    case {Map.get(data, "state"), stage} do
      {_, "allocating"} -> {:ok, :allocating}
      {_, "provisioning"} -> {:ok, :provisioning}
      {_, "booting"} -> {:ok, :booting}
      {state, _} when state != "starting" -> {:ok, :nil}
      _ -> {:error, "Invalid starting stage #{stage}"}
    end
  end

  @update_state_body_schema %{
    state: [
      type: :string,
      required: true,
      cast_func: &__MODULE__.cast_job_state/1
    ],
    stage: [
      type: :string,
      # required: fn _val, data -> data.state == "starting" end,
      required: false,
      cast_func: &__MODULE__.cast_job_starting_stage/2
    ],
    status_message: [type: :string],
    connection_info: [
      type: :any, # TODO
      required: false,
    ],
    # TODO: connection_info!
  }
  def update_state(conn, params) do
    IO.puts("Params: #{inspect params}, body params: #{inspect conn.body_params}")
    with {:ok, validated_job_id_params} <- Tarams.cast(params, @job_id_params_schema),
	 {:ok, validated_body} <- Tarams.cast(conn.body_params, @update_state_body_schema) do
      # TODO: validate the board's authentication token!
      Treadmill.Job.update_job_state(validated_job_id_params.id, validated_body)
      send_resp(conn, 204, "")
    else
      {:error, errors} ->
	IO.inspect(errors, label: "job update_state validation")
	conn
	|> put_status(:bad_request)
	|> put_view(TreadmillWeb.API.ErrorView)
	|> render("bad_request.json",
	  description: "Invalid request data: #{inspect errors}."
	)
    end
  end

  def put_console_log(conn, params) do
    with {:ok, validated_job_id_params} <- Tarams.cast(params, @job_id_params_schema),
         [offset_hdr_val] <- Plug.Conn.get_req_header(conn, "x-treadmill-console-offset"),
         {offset, ""} <- Integer.parse(offset_hdr_val, 10),
         [next_hdr_val] <- Plug.Conn.get_req_header(conn, "x-treadmill-console-next"),
         {next, ""} <- Integer.parse(next_hdr_val, 10),
	 {:ok, body, conn} = Plug.Conn.read_body(conn) do
      Treadmill.Job.put_console_log(validated_job_id_params.id, offset, next, body)
      send_resp(conn, 204, "")
    else
      {:error, errors} ->
	conn
	|> put_status(:bad_request)
	|> put_view(TreadmillWeb.API.ErrorView)
	|> render("bad_request.json",
	  description: "Invalid request data: #{inspect errors}."
	)
      [] ->
	conn
	|> put_status(:bad_request)
	|> put_view(TreadmillWeb.API.ErrorView)
	|> render("bad_request.json",
	  description: "Missing or invalid X-Treadmill-Console-Offset or X-Treadmill-Console-Next headers."
	)
      :error ->
	conn
	|> put_status(:bad_request)
	|> put_view(TreadmillWeb.API.ErrorView)
	|> render("bad_request.json",
	  description: "Missing or invalid X-Treadmill-Console-Offset or X-Treadmill-Console-Next headers."
	)
    end
  end
end
