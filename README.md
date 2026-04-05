# Process Control

Uma menu bar app em Tauri para mostrar quem está ocupando portas comuns de desenvolvimento e encerrar o culpado sem precisar abrir terminal, lembrar meia dúzia de comandos, ou fingir que gosta de `lsof`.

## O Que Faz

- monitora portas configuradas em `ports.json`
- aceita portas exatas e ranges no formato `"3000-4000,5432,6379"`
- atualiza a lista automaticamente
- recarrega a configuração em tempo real quando o arquivo muda
- encerra processo local com `kill -9`
- se a porta vier de container Docker publicado, usa `docker kill` em vez de matar o processo errado

## Como Usar

1. rode a app
2. clique no item da menubar
3. veja quais portas estão ocupadas
4. clique em `Edit` para abrir a configuração no VS Code
5. clique em `Encerrar processo` quando algum serviço resolver estragar seu dia

## Configuração

O arquivo `ports.json` é criado automaticamente na pasta de configuração da app.

Formato:

```json
{
  "ports": "3000-4000,5432,6379"
}
```

## Desenvolvimento

```bash
npm run tauri dev
```

Se alguma coisa estiver usando a porta errada, a app mostra.
Se for Docker, ela tenta não fazer cagada.
Se for processo local, ela mete `kill -9` sem terapia. 
