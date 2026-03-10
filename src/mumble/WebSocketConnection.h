// Copyright The Mumble Developers. All rights reserved.
// Use of this source code is governed by a BSD-style license
// that can be found in the LICENSE file at the root of the
// Mumble source tree or at <https://www.mumble.info/LICENSE>.

#ifndef MUMBLE_MUMBLE_WEBSOCKETCONNECTION_H_
#define MUMBLE_MUMBLE_WEBSOCKETCONNECTION_H_

#include "MumbleProtocol.h"
#include "crypto/CryptState.h"
#include "crypto/CryptStateOCB2.h"

#include <QtCore/QElapsedTimer>
#include <QtCore/QObject>
#include <QtNetwork/QHostAddress>
#include <QtNetwork/QSslCertificate>
#include <QtNetwork/QSslCipher>
#include <QtNetwork/QSslError>
#include <QtNetwork/QSslKey>
#include <QtWebSockets/QWebSocket>

#include <memory>

namespace google {
namespace protobuf {
	class Message;
}
} // namespace google

/// WebSocketConnection wraps a QWebSocket and exposes the same interface as
/// Connection so that ServerHandler can use it transparently when the server
/// address uses the ws:// or wss:// scheme.
///
/// The Mumble wire format over WebSocket is identical to the TCP framing:
///   [2-byte big-endian message type][4-byte big-endian length][payload]
/// Each WebSocket binary message carries exactly one such frame.
class WebSocketConnection : public QObject {
private:
	Q_OBJECT
	Q_DISABLE_COPY(WebSocketConnection)

	QWebSocket *m_socket;
	QElapsedTimer m_lastPacket;

protected slots:
	void socketBinaryMessageReceived(const QByteArray &msg);
	void socketError(QAbstractSocket::SocketError err);
	void socketDisconnected();
	void socketSslErrors(const QList< QSslError > &errors);

public:
	WebSocketConnection(QObject *parent, QWebSocket *socket);
	~WebSocketConnection() Q_DECL_OVERRIDE = default;

	static void messageToNetwork(const ::google::protobuf::Message &msg, Mumble::Protocol::TCPMessageType msgType,
								 QByteArray &cache);
	void sendMessage(const ::google::protobuf::Message &msg, Mumble::Protocol::TCPMessageType msgType,
					 QByteArray &cache);
	void sendMessage(const QByteArray &data);
	void disconnectSocket(bool force = false);
	void forceFlush();
	void proceedAnyway();
	qint64 activityTime() const;
	void resetActivityTime();

	QHostAddress peerAddress() const;
	quint16 peerPort() const;
	QHostAddress localAddress() const;
	quint16 localPort() const;
	QList< QSslCertificate > peerCertificateChain() const;
	QSslCipher sessionCipher() const;
	QSsl::SslProtocol sessionProtocol() const;
	QString sessionProtocolString() const;
	QSslKey ephemeralServerKey() const;

	std::unique_ptr< CryptState > csCrypt;
	bool bDisconnectedEmitted;

signals:
	void encrypted();
	void connectionClosed(QAbstractSocket::SocketError, const QString &reason);
	void message(Mumble::Protocol::TCPMessageType type, const QByteArray &);
	void handleSslErrors(const QList< QSslError > &);
};

using WebSocketConnectionPtr = std::shared_ptr< WebSocketConnection >;

#endif // MUMBLE_MUMBLE_WEBSOCKETCONNECTION_H_
